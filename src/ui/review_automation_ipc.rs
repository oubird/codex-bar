//! Process-isolated client and wire protocol for Codex UI Automation.
//!
//! Chromium's UIA provider is an out-of-process synchronous COM service.  A
//! provider call can therefore block indefinitely.  The resident Win32 UI
//! must never make those calls itself: this module runs the existing UIA
//! engine in a disposable copy of `codexplus.exe` and enforces a hard deadline
//! from a broker thread.

#![cfg(windows)]

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{CloseHandle, HWND, RECT, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};

use super::review_automation::{ActionButtonState, ReviewAutomation as LocalAutomation};

// A cold helper launch plus one normal Chromium accessibility-tree walk is
// roughly 700-800 ms on the supported desktop build.  Keep a bounded margin
// above that baseline while still terminating a genuinely stuck provider
// quickly enough that it cannot create a persistent cross-process hang.
const REQUEST_TIMEOUT: Duration = Duration::from_millis(1_500);
const NORMAL_PROBE_INTERVAL: Duration = Duration::from_millis(500);
const MAX_CACHED_UNAVAILABLE_SCANS: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum HelperRequest {
    Inspect { id: u64, hwnd: i64 },
    BeginPrompt { id: u64, hwnd: i64, prompt: String },
    BeginPlan { id: u64, hwnd: i64 },
    BeginReview { id: u64, hwnd: i64 },
    Advance { id: u64 },
}

impl HelperRequest {
    fn id(&self) -> u64 {
        match self {
            Self::Inspect { id, .. }
            | Self::BeginPrompt { id, .. }
            | Self::BeginPlan { id, .. }
            | Self::BeginReview { id, .. }
            | Self::Advance { id } => *id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum HelperStatus {
    Ok,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct WireRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl From<RECT> for WireRect {
    fn from(value: RECT) -> Self {
        Self {
            left: value.left,
            top: value.top,
            right: value.right,
            bottom: value.bottom,
        }
    }
}

impl From<WireRect> for RECT {
    fn from(value: WireRect) -> Self {
        Self {
            left: value.left,
            top: value.top,
            right: value.right,
            bottom: value.bottom,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct WireButtonState {
    prompt_rect: WireRect,
    plan_rect: WireRect,
    review_rect: WireRect,
    enabled: bool,
}

impl From<ActionButtonState> for WireButtonState {
    fn from(value: ActionButtonState) -> Self {
        Self {
            prompt_rect: value.prompt_rect.into(),
            plan_rect: value.plan_rect.into(),
            review_rect: value.review_rect.into(),
            enabled: value.enabled,
        }
    }
}

impl From<WireButtonState> for ActionButtonState {
    fn from(value: WireButtonState) -> Self {
        Self {
            prompt_rect: value.prompt_rect.into(),
            plan_rect: value.plan_rect.into(),
            review_rect: value.review_rect.into(),
            enabled: value.enabled,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HelperResponse {
    id: u64,
    status: HelperStatus,
    pending: bool,
    buttons: Option<WireButtonState>,
}

enum BrokerCommand {
    Request(HelperRequest),
    Shutdown,
}

#[derive(Debug)]
struct BrokerReply {
    id: u64,
    response: Option<HelperResponse>,
    timed_out: bool,
}

struct HelperProcess {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    responses: Receiver<Result<HelperResponse, ()>>,
}

impl HelperProcess {
    fn spawn() -> Result<Self, ()> {
        let executable = std::env::current_exe().map_err(|_| ())?;
        let mut child = Command::new(executable)
            .arg("--uia-helper")
            .arg(std::process::id().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| ())?;
        let stdin = child.stdin.take().ok_or(())?;
        let stdout = child.stdout.take().ok_or(())?;
        let (response_sender, responses) = mpsc::channel();
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let response = line
                    .map_err(|_| ())
                    .and_then(|line| serde_json::from_str(&line).map_err(|_| ()));
                let failed = response.is_err();
                if response_sender.send(response).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            stdin: BufWriter::new(stdin),
            responses,
        })
    }

    fn request(&mut self, request: &HelperRequest) -> Result<HelperResponse, bool> {
        serde_json::to_writer(&mut self.stdin, request).map_err(|_| false)?;
        self.stdin.write_all(b"\n").map_err(|_| false)?;
        self.stdin.flush().map_err(|_| false)?;
        match self.responses.recv_timeout(REQUEST_TIMEOUT) {
            Ok(Ok(response)) if response.id == request.id() => Ok(response),
            Ok(_) | Err(RecvTimeoutError::Disconnected) => Err(false),
            Err(RecvTimeoutError::Timeout) => Err(true),
        }
    }

    fn terminate(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for HelperProcess {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn start_broker() -> (Sender<BrokerCommand>, Receiver<BrokerReply>) {
    let (command_sender, command_receiver) = mpsc::channel();
    let (reply_sender, reply_receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut helper: Option<HelperProcess> = None;
        while let Ok(command) = command_receiver.recv() {
            let BrokerCommand::Request(request) = command else {
                break;
            };
            if helper.is_none() {
                helper = HelperProcess::spawn().ok();
            }
            let result = helper.as_mut().map(|child| child.request(&request));
            let reply = match result {
                Some(Ok(response)) => BrokerReply {
                    id: request.id(),
                    response: Some(response),
                    timed_out: false,
                },
                Some(Err(timed_out)) => {
                    if let Some(mut child) = helper.take() {
                        child.terminate();
                    }
                    BrokerReply {
                        id: request.id(),
                        response: None,
                        timed_out,
                    }
                }
                None => BrokerReply {
                    id: request.id(),
                    response: None,
                    timed_out: false,
                },
            };
            if reply_sender.send(reply).is_err() {
                break;
            }
        }
        if let Some(mut child) = helper {
            child.terminate();
        }
    });
    (command_sender, reply_receiver)
}

#[derive(Debug)]
enum QueuedAction {
    Prompt { target: i64, prompt: String },
    Plan { target: i64 },
    Review { target: i64 },
}

#[derive(Debug, Clone, Copy)]
enum InFlightKind {
    Inspect { target: i64 },
    Action,
}

#[derive(Debug, Clone, Copy)]
struct InFlight {
    id: u64,
    kind: InFlightKind,
}

/// Non-blocking facade used by the resident Win32 message thread.
pub struct RemoteReviewAutomation {
    commands: Sender<BrokerCommand>,
    replies: Receiver<BrokerReply>,
    next_id: u64,
    in_flight: Option<InFlight>,
    queued_action: Option<QueuedAction>,
    action_active: bool,
    current_target: Option<i64>,
    buttons: Option<ActionButtonState>,
    next_probe_at: Instant,
    consecutive_timeouts: u32,
    consecutive_unavailable_scans: u32,
}

impl RemoteReviewAutomation {
    pub fn new() -> Self {
        let (commands, replies) = start_broker();
        Self {
            commands,
            replies,
            next_id: 1,
            in_flight: None,
            queued_action: None,
            action_active: false,
            current_target: None,
            buttons: None,
            next_probe_at: Instant::now(),
            consecutive_timeouts: 0,
            consecutive_unavailable_scans: 0,
        }
    }

    /// Poll completed IPC and schedule at most one new helper request.
    /// This method never waits for the helper process.
    pub fn tick(&mut self, target: Option<HWND>, target_is_foreground: bool) {
        let target_value = target.map(hwnd_value);
        if target_value != self.current_target {
            self.current_target = target_value;
            self.buttons = None;
            self.consecutive_unavailable_scans = 0;
            self.next_probe_at = Instant::now();
        }
        self.drain_replies();

        if self.in_flight.is_some() {
            return;
        }
        if let Some(action) = self.queued_action.take() {
            let request = match action {
                QueuedAction::Prompt { target, prompt } => HelperRequest::BeginPrompt {
                    id: self.take_id(),
                    hwnd: target,
                    prompt,
                },
                QueuedAction::Plan { target } => HelperRequest::BeginPlan {
                    id: self.take_id(),
                    hwnd: target,
                },
                QueuedAction::Review { target } => HelperRequest::BeginReview {
                    id: self.take_id(),
                    hwnd: target,
                },
            };
            self.send(request, InFlightKind::Action);
        } else if self.action_active {
            let request = HelperRequest::Advance { id: self.take_id() };
            self.send(request, InFlightKind::Action);
        } else if target_is_foreground && Instant::now() >= self.next_probe_at {
            if let Some(target) = target_value {
                let request = HelperRequest::Inspect {
                    id: self.take_id(),
                    hwnd: target,
                };
                self.send(request, InFlightKind::Inspect { target });
            }
        }
    }

    pub fn button_state(&self) -> Option<ActionButtonState> {
        self.buttons
    }

    pub fn begin_prompt(&mut self, target: HWND, prompt: &str) -> bool {
        let prompt = prompt.trim();
        if prompt.is_empty() || prompt.contains(['\r', '\n']) || self.action_active {
            return false;
        }
        self.queue_action(QueuedAction::Prompt {
            target: hwnd_value(target),
            prompt: prompt.to_owned(),
        })
    }

    pub fn begin_plan(&mut self, target: HWND) -> bool {
        if self.action_active {
            return false;
        }
        self.queue_action(QueuedAction::Plan {
            target: hwnd_value(target),
        })
    }

    pub fn begin_uncommitted_review(&mut self, target: HWND) -> bool {
        if self.action_active {
            return false;
        }
        self.queue_action(QueuedAction::Review {
            target: hwnd_value(target),
        })
    }

    pub fn is_pending(&self) -> bool {
        self.action_active
    }

    fn queue_action(&mut self, action: QueuedAction) -> bool {
        self.action_active = true;
        self.queued_action = Some(action);
        self.buttons = None;
        true
    }

    fn drain_replies(&mut self) {
        while let Ok(reply) = self.replies.try_recv() {
            self.apply_reply(reply);
        }
    }

    fn apply_reply(&mut self, reply: BrokerReply) {
        let Some(in_flight) = self.in_flight else {
            return;
        };
        if !response_matches(in_flight.id, reply.id) {
            return;
        }
        self.in_flight = None;
        let Some(response) = reply.response else {
            self.buttons = None;
            self.action_active = false;
            self.queued_action = None;
            self.consecutive_unavailable_scans = 0;
            if reply.timed_out {
                self.consecutive_timeouts = self.consecutive_timeouts.saturating_add(1);
                self.next_probe_at = Instant::now() + recovery_delay(self.consecutive_timeouts);
            } else {
                self.next_probe_at = Instant::now() + Duration::from_secs(2);
            }
            return;
        };

        match in_flight.kind {
            InFlightKind::Inspect { target } => {
                if Some(target) != self.current_target {
                    return;
                }
                if response.status == HelperStatus::Ok {
                    self.buttons = response.buttons.map(Into::into);
                    self.consecutive_unavailable_scans = 0;
                } else {
                    self.consecutive_unavailable_scans =
                        self.consecutive_unavailable_scans.saturating_add(1);
                    if self.consecutive_unavailable_scans <= MAX_CACHED_UNAVAILABLE_SCANS {
                        if let Some(buttons) = self.buttons.as_mut() {
                            // Allow one transient React accessibility-tree
                            // commit without moving the controls. They remain
                            // visible but cannot be clicked until confirmed.
                            buttons.enabled = false;
                        }
                    } else {
                        self.buttons = None;
                    }
                }
                self.consecutive_timeouts = 0;
                self.next_probe_at = Instant::now() + NORMAL_PROBE_INTERVAL;
            }
            InFlightKind::Action => {
                if response.status == HelperStatus::Ok {
                    self.action_active = response.pending;
                } else {
                    self.action_active = false;
                }
                self.buttons = None;
                self.consecutive_unavailable_scans = 0;
                if !self.action_active {
                    self.next_probe_at = Instant::now() + NORMAL_PROBE_INTERVAL;
                }
            }
        }
    }

    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    fn send(&mut self, request: HelperRequest, kind: InFlightKind) {
        let id = request.id();
        if self.commands.send(BrokerCommand::Request(request)).is_ok() {
            self.in_flight = Some(InFlight { id, kind });
        } else {
            self.buttons = None;
            self.action_active = false;
            self.consecutive_unavailable_scans = 0;
            self.next_probe_at = Instant::now() + Duration::from_secs(10);
        }
    }
}

impl Default for RemoteReviewAutomation {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RemoteReviewAutomation {
    fn drop(&mut self) {
        let _ = self.commands.send(BrokerCommand::Shutdown);
    }
}

fn hwnd_value(hwnd: HWND) -> i64 {
    hwnd.0 as isize as i64
}

fn hwnd_from_value(value: i64) -> HWND {
    HWND(value as isize as *mut _)
}

fn response_matches(expected: u64, actual: u64) -> bool {
    expected == actual
}

fn recovery_delay(consecutive_timeouts: u32) -> Duration {
    match consecutive_timeouts {
        0 | 1 => Duration::from_secs(2),
        2 => Duration::from_secs(5),
        _ => Duration::from_secs(10),
    }
}

/// Entry point for the disposable UIA worker process.
pub fn run_helper(parent_pid: Option<u32>) -> i32 {
    if let Some(parent_pid) = parent_pid {
        start_parent_watchdog(parent_pid);
    }
    let mut automation = LocalAutomation::new();
    let stdin = std::io::stdin();
    let mut stdout = BufWriter::new(std::io::stdout());
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            return 2;
        };
        let Ok(request) = serde_json::from_str::<HelperRequest>(&line) else {
            return 2;
        };
        let response = execute_request(&mut automation, request);
        if serde_json::to_writer(&mut stdout, &response).is_err()
            || stdout.write_all(b"\n").is_err()
            || stdout.flush().is_err()
        {
            return 3;
        }
    }
    0
}

fn execute_request(automation: &mut LocalAutomation, request: HelperRequest) -> HelperResponse {
    match request {
        HelperRequest::Inspect { id, hwnd } => {
            let buttons = automation.inspect_buttons(hwnd_from_value(hwnd));
            HelperResponse {
                id,
                status: if buttons.is_some() {
                    HelperStatus::Ok
                } else {
                    HelperStatus::Unavailable
                },
                pending: automation.is_pending(),
                buttons: buttons.map(Into::into),
            }
        }
        HelperRequest::BeginPrompt { id, hwnd, prompt } => action_response(
            id,
            automation.begin_prompt(hwnd_from_value(hwnd), &prompt),
            automation,
        ),
        HelperRequest::BeginPlan { id, hwnd } => {
            action_response(id, automation.begin_plan(hwnd_from_value(hwnd)), automation)
        }
        HelperRequest::BeginReview { id, hwnd } => action_response(
            id,
            automation.begin_uncommitted_review(hwnd_from_value(hwnd)),
            automation,
        ),
        HelperRequest::Advance { id } => {
            let pending = automation.advance();
            HelperResponse {
                id,
                status: HelperStatus::Ok,
                pending,
                buttons: None,
            }
        }
    }
}

fn action_response(id: u64, accepted: bool, automation: &LocalAutomation) -> HelperResponse {
    HelperResponse {
        id,
        status: if accepted {
            HelperStatus::Ok
        } else {
            HelperStatus::Unavailable
        },
        pending: accepted && automation.is_pending(),
        buttons: None,
    }
}

fn start_parent_watchdog(parent_pid: u32) {
    thread::spawn(move || unsafe {
        let Ok(parent) = OpenProcess(PROCESS_SYNCHRONIZE, false, parent_pid) else {
            std::process::exit(0);
        };
        let wait = WaitForSingleObject(parent, u32::MAX);
        let _ = CloseHandle(parent);
        if wait == WAIT_OBJECT_0 {
            std::process::exit(0);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_remote() -> (
        RemoteReviewAutomation,
        Receiver<BrokerCommand>,
        Sender<BrokerReply>,
    ) {
        let (commands, command_receiver) = mpsc::channel();
        let (reply_sender, replies) = mpsc::channel();
        (
            RemoteReviewAutomation {
                commands,
                replies,
                next_id: 1,
                in_flight: None,
                queued_action: None,
                action_active: false,
                current_target: None,
                buttons: None,
                next_probe_at: Instant::now(),
                consecutive_timeouts: 0,
                consecutive_unavailable_scans: 0,
            },
            command_receiver,
            reply_sender,
        )
    }

    fn test_hwnd(value: isize) -> HWND {
        HWND(value as *mut _)
    }

    #[test]
    fn timeout_backoff_is_bounded() {
        assert_eq!(recovery_delay(1), Duration::from_secs(2));
        assert_eq!(recovery_delay(2), Duration::from_secs(5));
        assert_eq!(recovery_delay(3), Duration::from_secs(10));
        assert_eq!(recovery_delay(100), Duration::from_secs(10));
    }

    #[test]
    fn normal_scans_run_twice_per_second() {
        assert_eq!(NORMAL_PROBE_INTERVAL, Duration::from_millis(500));
    }

    #[test]
    fn stale_responses_are_rejected() {
        assert!(response_matches(42, 42));
        assert!(!response_matches(42, 41));
    }

    #[test]
    fn only_one_probe_can_be_in_flight() {
        let (mut remote, commands, _replies) = test_remote();
        remote.tick(Some(test_hwnd(123)), true);
        assert!(matches!(
            commands.try_recv(),
            Ok(BrokerCommand::Request(HelperRequest::Inspect { id: 1, .. }))
        ));
        remote.tick(Some(test_hwnd(123)), true);
        assert!(commands.try_recv().is_err());
        assert_eq!(remote.in_flight.map(|request| request.id), Some(1));
    }

    #[test]
    fn timeout_hides_buttons_and_enters_recovery_delay() {
        let (mut remote, commands, replies) = test_remote();
        remote.tick(Some(test_hwnd(123)), true);
        let _ = commands.try_recv().unwrap();
        remote.buttons = Some(ActionButtonState {
            prompt_rect: RECT::default(),
            plan_rect: RECT::default(),
            review_rect: RECT::default(),
            enabled: true,
        });
        replies
            .send(BrokerReply {
                id: 1,
                response: None,
                timed_out: true,
            })
            .unwrap();

        remote.tick(Some(test_hwnd(123)), true);

        assert!(remote.buttons.is_none());
        assert_eq!(remote.consecutive_timeouts, 1);
        assert!(remote.next_probe_at > Instant::now());
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn action_clicked_during_probe_runs_after_probe_response() {
        let (mut remote, commands, replies) = test_remote();
        remote.tick(Some(test_hwnd(123)), true);
        let _ = commands.try_recv().unwrap();
        assert!(remote.begin_plan(test_hwnd(123)));
        replies
            .send(BrokerReply {
                id: 1,
                response: Some(HelperResponse {
                    id: 1,
                    status: HelperStatus::Unavailable,
                    pending: false,
                    buttons: None,
                }),
                timed_out: false,
            })
            .unwrap();

        remote.tick(Some(test_hwnd(123)), true);

        assert!(matches!(
            commands.try_recv(),
            Ok(BrokerCommand::Request(HelperRequest::BeginPlan {
                id: 2,
                ..
            }))
        ));
        assert!(remote.is_pending());
    }

    #[test]
    fn transient_unavailable_scan_keeps_cached_geometry_disabled() {
        let (mut remote, commands, replies) = test_remote();
        remote.current_target = Some(123);
        remote.buttons = Some(ActionButtonState {
            prompt_rect: RECT {
                left: 10,
                top: 20,
                right: 38,
                bottom: 48,
            },
            plan_rect: RECT::default(),
            review_rect: RECT::default(),
            enabled: true,
        });
        remote.tick(Some(test_hwnd(123)), true);
        let _ = commands.try_recv().unwrap();
        replies
            .send(BrokerReply {
                id: 1,
                response: Some(HelperResponse {
                    id: 1,
                    status: HelperStatus::Unavailable,
                    pending: false,
                    buttons: None,
                }),
                timed_out: false,
            })
            .unwrap();

        remote.tick(Some(test_hwnd(123)), true);

        let buttons = remote.button_state().unwrap();
        assert_eq!(buttons.prompt_rect.left, 10);
        assert!(!buttons.enabled);
        assert_eq!(remote.consecutive_unavailable_scans, 1);
    }

    #[test]
    fn repeated_unavailable_scans_hide_cached_geometry() {
        let (mut remote, commands, replies) = test_remote();
        remote.current_target = Some(123);
        remote.buttons = Some(ActionButtonState {
            prompt_rect: RECT::default(),
            plan_rect: RECT::default(),
            review_rect: RECT::default(),
            enabled: true,
        });

        for id in 1..=2 {
            remote.next_probe_at = Instant::now();
            remote.tick(Some(test_hwnd(123)), true);
            let _ = commands.try_recv().unwrap();
            replies
                .send(BrokerReply {
                    id,
                    response: Some(HelperResponse {
                        id,
                        status: HelperStatus::Unavailable,
                        pending: false,
                        buttons: None,
                    }),
                    timed_out: false,
                })
                .unwrap();
            remote.tick(Some(test_hwnd(123)), true);
        }

        assert!(remote.button_state().is_none());
        assert_eq!(remote.consecutive_unavailable_scans, 2);
    }

    #[test]
    fn protocol_round_trip_preserves_large_window_handles() {
        let request = HelperRequest::BeginPlan {
            id: 7,
            hwnd: 0x0000_7fff_ffff_abcd,
        };
        let encoded = serde_json::to_string(&request).unwrap();
        let decoded: HelperRequest = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.id(), 7);
        match decoded {
            HelperRequest::BeginPlan { hwnd, .. } => {
                assert_eq!(hwnd, 0x0000_7fff_ffff_abcd)
            }
            _ => panic!("wrong request variant"),
        }
    }
}
