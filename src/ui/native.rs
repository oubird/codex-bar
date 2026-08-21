//! Hidden Win32 host window and resident application lifecycle.

#![cfg(windows)]

use std::sync::mpsc::Receiver;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, SYSTEMTIME, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Time::{
    GetTimeZoneInformation, SystemTimeToTzSpecificLocalTime, TIME_ZONE_ID_INVALID,
    TIME_ZONE_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowLongPtrW, LoadIconW, PostQuitMessage, RegisterClassExW, SetForegroundWindow, SetTimer,
    SetWindowLongPtrW, ShowWindow, TranslateMessage, GWLP_USERDATA, HICON, IDI_APPLICATION, MSG,
    SW_HIDE, WINDOW_STYLE, WM_COMMAND, WM_DESTROY, WM_TIMER, WNDCLASSEXW,
};

use super::app_state::{AppState, RefreshDisposition};
use super::overlay::OverlayHandle;
use super::poll::{self, PollOutcome, PollSender};
use super::prompt_menu;
use super::review_automation_ipc::RemoteReviewAutomation as ReviewAutomation;
use super::review_button::{
    ActionButtonHandle, QuickAction, WM_APP_PLAN_CLICKED, WM_APP_PROMPT_CLICKED,
    WM_APP_REVIEW_CLICKED,
};
use super::settings::{self as settings_window, WM_APP_SETTINGS_CHANGED};
use super::target_window;
use super::tray::{
    TrayIcon, IDM_DIAGNOSE, IDM_EXIT, IDM_REFRESH, IDM_SETTINGS, IDM_TOGGLE_OVERLAY, WM_APP_TRAY,
};
use crate::formatting::DisplayOptions;
use crate::settings::{self, SettingsPatch};

const HOST_CLASS: &str = "CodexPlusHostWindow";
const TIMER_RECONCILE: usize = 1;
const TIMER_POLL: usize = 2;
const TIMER_COUNTDOWN: usize = 3;

pub fn wide_str(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn module_instance() -> HINSTANCE {
    unsafe {
        GetModuleHandleW(None)
            .map(|module| HINSTANCE(module.0))
            .unwrap_or_default()
    }
}

pub fn load_app_icon() -> HICON {
    unsafe {
        LoadIconW(module_instance(), IDI_APPLICATION).unwrap_or_else(|_| {
            LoadIconW(HINSTANCE::default(), IDI_APPLICATION).unwrap_or_default()
        })
    }
}

/// Difference between local time and UTC, expressed in seconds east of UTC.
/// Uses `GetTimeZoneInformation` + `SystemTimeToTzSpecificLocalTime` so the
/// result already factors in the currently-active DST rule. Returns `0` on
/// any conversion failure so the caller falls back to rendering in UTC.
pub fn local_shift_seconds(utc_unix: i64) -> i64 {
    let utc_system = unix_to_system_time(utc_unix);
    let mut tz = TIME_ZONE_INFORMATION::default();
    unsafe {
        // `GetTimeZoneInformation` only returns `TIME_ZONE_ID_INVALID` on
        // failure; it returns `TIME_ZONE_ID_UNKNOWN`/`STANDARD`/`DAYLIGHT`
        // (all non-zero) when the call succeeds. Treating any non-invalid
        // result as success lets the populated `tz` struct reach
        // `SystemTimeToTzSpecificLocalTime`, which itself re-applies the
        // Bias/StandardBias/DaylightBias rules. Failing here returns 0 so
        // the caller can fall back to a UTC render without mis-rendering.
        let result = GetTimeZoneInformation(&mut tz);
        if result == TIME_ZONE_ID_INVALID {
            return 0;
        }
        let mut local = SYSTEMTIME::default();
        if SystemTimeToTzSpecificLocalTime(None, &utc_system, &mut local).is_err() {
            return 0;
        }
        let Some(local_unix) = system_time_to_unix(&local) else {
            return 0;
        };
        local_unix.saturating_sub(utc_unix)
    }
}

fn unix_to_system_time(unix: i64) -> SYSTEMTIME {
    let days = unix.div_euclid(86_400);
    let secs_of_day = unix.rem_euclid(86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;
    SYSTEMTIME {
        wYear: year as u16,
        wMonth: month as u16,
        wDay: day as u16,
        wHour: hour as u16,
        wMinute: minute as u16,
        wSecond: second as u16,
        wMilliseconds: 0,
        wDayOfWeek: 0,
    }
}

fn system_time_to_unix(value: &SYSTEMTIME) -> Option<i64> {
    let year = value.wYear as i64;
    let month = value.wMonth;
    let day = value.wDay;
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400
        + value.wHour as i64 * 3600
        + value.wMinute as i64 * 60
        + value.wSecond as i64;
    Some(secs)
}

/// Howard Hinnant's date algorithm. Mirror of `formatting::civil_from_days`;
/// duplicated here so this utility does not pull the formatting module into
/// native code that is allowed to bind to Win32.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 {
        (mp + 3) as u32
    } else {
        (mp - 9) as u32
    };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m, d)
}

pub fn overlay_extended_styles_bits() -> u32 {
    // WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT.
    // Deliberately omit WS_EX_TOPMOST: the overlay is shown only while Codex is
    // foreground and must not float above unrelated applications.
    0x00000080 | 0x08000000 | 0x00080000 | 0x00000020
}

pub fn window_rect(hwnd: HWND) -> Option<RECT> {
    unsafe {
        let mut rect = RECT::default();
        windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut rect).ok()?;
        Some(rect)
    }
}

pub fn is_window_visible(hwnd: HWND) -> bool {
    unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd).as_bool() }
}

pub fn class_name(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 256];
        let len = windows::Win32::UI::WindowsAndMessaging::GetClassNameW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

pub fn root_owner(hwnd: HWND) -> HWND {
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetAncestor(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::GA_ROOT,
        )
    }
}

pub fn foreground_window() -> HWND {
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() }
}

pub fn window_thread_process_id(hwnd: HWND) -> (u32, u32) {
    let mut pid = 0u32;
    let tid = unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(hwnd, Some(&mut pid))
    };
    (tid, pid)
}

pub fn enumerate_top_level() -> Vec<HWND> {
    struct State(Vec<HWND>);
    unsafe extern "system" fn callback(
        hwnd: HWND,
        lparam: LPARAM,
    ) -> windows::Win32::Foundation::BOOL {
        let state = &mut *(lparam.0 as *mut State);
        if !hwnd.is_invalid() {
            state.0.push(hwnd);
        }
        windows::Win32::Foundation::BOOL(1)
    }
    let mut state = State(Vec::new());
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::EnumWindows(
            Some(callback),
            LPARAM(&mut state as *mut _ as isize),
        );
    }
    state.0
}

struct AppRuntime {
    state: AppState,
    overlay: OverlayHandle,
    prompt_button: ActionButtonHandle,
    plan_button: ActionButtonHandle,
    review_button: ActionButtonHandle,
    review_automation: ReviewAutomation,
    tray: TrayIcon,
    poll_sender: PollSender,
    poll_receiver: Receiver<PollOutcome>,
    last_target: Option<target_window::WindowIdentity>,
}

impl AppRuntime {
    fn new(hwnd: HWND) -> Result<Self, String> {
        let settings = settings::load();
        let cached = crate::storage::load();
        let initial_display = DisplayOptions::from(&settings);
        let state = AppState::new(settings, cached);
        let overlay = OverlayHandle::create()?;
        let prompt_button = ActionButtonHandle::create(hwnd, QuickAction::Prompt)?;
        let plan_button = ActionButtonHandle::create(hwnd, QuickAction::Plan)?;
        let review_button = ActionButtonHandle::create(hwnd, QuickAction::Review)?;
        let review_automation = ReviewAutomation::new();
        let mut tray = TrayIcon::new(hwnd);
        tray.add_or_update(&state.state, initial_display);
        let (poll_sender, poll_receiver) = poll::start();
        let mut runtime = Self {
            state,
            overlay,
            prompt_button,
            plan_button,
            review_button,
            review_automation,
            tray,
            poll_sender,
            poll_receiver,
            last_target: None,
        };
        runtime.request_refresh();
        Ok(runtime)
    }

    fn process_results(&mut self, hwnd: HWND) {
        while let Ok(outcome) = self.poll_receiver.try_recv() {
            let disposition = match outcome {
                PollOutcome::Success(snapshot) => {
                    let _ = crate::storage::save(&snapshot);
                    self.state.apply_success(snapshot)
                }
                PollOutcome::Failure(error) => self.state.apply_failure(error),
            };
            self.overlay
                .set_state(self.state.state.clone(), self.display_options());
            self.tray
                .add_or_update(&self.state.state, self.display_options());
            let delay_ms = match disposition {
                RefreshDisposition::Success => self
                    .state
                    .settings
                    .refresh_interval_secs
                    .saturating_mul(1000),
                RefreshDisposition::RetryAfterSeconds(seconds) => seconds.saturating_mul(1000),
                RefreshDisposition::WaitForCredentialChange => 900_000,
            };
            unsafe {
                let _ = SetTimer(hwnd, TIMER_POLL, delay_ms, None);
            }
            if self.state.take_pending_manual_refresh() {
                self.request_refresh();
            }
        }
    }

    fn request_refresh(&mut self) {
        if self.state.begin_refresh(true) {
            self.poll_sender
                .request_refresh(self.state.settings.proxy_policy());
        }
    }

    fn sync_overlay(&mut self) {
        if let Some(foreground) = target_window::foreground_codex_window() {
            self.last_target = Some(foreground);
        } else if let Some(previous) = self.last_target.as_ref() {
            self.last_target = target_window::revalidate_target(previous);
        }

        let target = self
            .last_target
            .as_ref()
            .map(|identity| HWND(identity.hwnd as *mut _));
        let target_is_foreground = target
            .is_some_and(|candidate| root_owner(foreground_window()) == root_owner(candidate));
        self.overlay.sync_to_target(
            target,
            self.state.settings.overlay_enabled,
            target_is_foreground,
        );
        self.sync_action_buttons(target, target_is_foreground);
        self.tray
            .add_or_update(&self.state.state, self.display_options());
    }

    fn sync_action_buttons(&mut self, target: Option<HWND>, target_is_foreground: bool) {
        self.review_automation.tick(target, target_is_foreground);
        let Some(target) = target else {
            self.prompt_button.hide();
            self.plan_button.hide();
            self.review_button.hide();
            return;
        };
        if !target_is_foreground || self.review_automation.is_pending() {
            self.prompt_button.hide();
            self.plan_button.hide();
            self.review_button.hide();
            return;
        }
        let Some(buttons) = self.review_automation.button_state() else {
            self.prompt_button.hide();
            self.plan_button.hide();
            self.review_button.hide();
            return;
        };
        self.prompt_button.sync(
            target,
            buttons.prompt_rect,
            buttons.enabled && !self.state.settings.quick_prompts.is_empty(),
            self.state.settings.language,
        );
        self.plan_button.sync(
            target,
            buttons.plan_rect,
            buttons.enabled,
            self.state.settings.language,
        );
        self.review_button.sync(
            target,
            buttons.review_rect,
            buttons.enabled,
            self.state.settings.language,
        );
    }

    fn start_prompt_menu(&mut self) {
        let Some(target) = self
            .last_target
            .as_ref()
            .map(|identity| HWND(identity.hwnd as *mut _))
        else {
            return;
        };
        if root_owner(foreground_window()) != root_owner(target)
            || self.state.settings.quick_prompts.is_empty()
        {
            return;
        }
        let Some(anchor) = self.prompt_button.screen_rect() else {
            return;
        };
        let prompts = self.state.settings.quick_prompts.clone();
        let Some(selected) =
            prompt_menu::select_prompt(self.prompt_button.hwnd(), anchor, &prompts)
        else {
            return;
        };
        let Some(prompt) = prompts.get(selected) else {
            return;
        };
        let _ = unsafe { SetForegroundWindow(target) };
        if self.review_automation.begin_prompt(target, prompt) {
            self.hide_action_buttons();
        }
    }

    fn hide_action_buttons(&self) {
        self.prompt_button.hide();
        self.plan_button.hide();
        self.review_button.hide();
    }

    fn start_plan(&mut self) {
        let Some(target) = self
            .last_target
            .as_ref()
            .map(|identity| HWND(identity.hwnd as *mut _))
        else {
            return;
        };
        if root_owner(foreground_window()) != root_owner(target) {
            return;
        }
        if self.review_automation.begin_plan(target) {
            self.hide_action_buttons();
        }
    }

    fn start_uncommitted_review(&mut self) {
        let Some(target) = self
            .last_target
            .as_ref()
            .map(|identity| HWND(identity.hwnd as *mut _))
        else {
            return;
        };
        if root_owner(foreground_window()) != root_owner(target) {
            return;
        }
        if self.review_automation.begin_uncommitted_review(target) {
            self.hide_action_buttons();
        }
    }

    fn display_options(&self) -> DisplayOptions {
        DisplayOptions::from(&self.state.settings)
    }

    fn toggle_overlay(&mut self) {
        SettingsPatch {
            overlay_enabled: Some(!self.state.settings.overlay_enabled),
            ..Default::default()
        }
        .apply_to(&mut self.state.settings);
        let _ = settings::save(&self.state.settings);
        self.sync_overlay();
    }

    fn shutdown(&mut self) {
        self.tray.remove();
        self.prompt_button.destroy();
        self.plan_button.destroy();
        self.review_button.destroy();
        self.overlay.destroy();
    }
}

pub fn run_app() -> Result<(), String> {
    unsafe {
        let class_name = wide_str(HOST_CLASS);
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(host_proc),
            hInstance: HINSTANCE::default(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassExW(&class);
        let hwnd = CreateWindowExW(
            Default::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR::null(),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            HWND::default(),
            None,
            HINSTANCE::default(),
            None,
        )
        .map_err(|_| "Could not create CodexPlus host window.".to_string())?;
        let runtime = Box::new(AppRuntime::new(hwnd)?);
        let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(runtime) as isize);
        let _ = ShowWindow(hwnd, SW_HIDE);
        let _ = SetTimer(hwnd, TIMER_RECONCILE, 500, None);
        let _ = SetTimer(hwnd, TIMER_COUNTDOWN, 1000, None);
        let mut message = MSG::default();
        while GetMessageW(&mut message, HWND::default(), 0, 0).as_bool() {
            if settings_window::pre_translate_message(&message) {
                continue;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        Ok(())
    }
}

unsafe extern "system" fn host_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_TIMER => {
            if let Some(runtime) = runtime_for(hwnd) {
                match wparam.0 {
                    TIMER_RECONCILE => runtime.sync_overlay(),
                    TIMER_POLL => runtime.request_refresh(),
                    TIMER_COUNTDOWN => {
                        let display = DisplayOptions::from(&runtime.state.settings);
                        runtime
                            .overlay
                            .set_state(runtime.state.state.clone(), display);
                        runtime.tray.add_or_update(&runtime.state.state, display);
                    }
                    _ => {}
                }
                runtime.process_results(hwnd);
            }
            LRESULT(0)
        }
        WM_APP_TRAY => {
            if let Some(runtime) = runtime_for(hwnd) {
                // Right click opens the native menu. Tray commands reach WM_COMMAND.
                if lparam.0 as u32 == windows::Win32::UI::WindowsAndMessaging::WM_RBUTTONUP {
                    runtime.tray.show_menu(
                        runtime.state.settings.overlay_enabled,
                        &runtime.state.state,
                        DisplayOptions::from(&runtime.state.settings),
                    );
                }
            }
            LRESULT(0)
        }
        WM_APP_SETTINGS_CHANGED => {
            if let Some(runtime) = runtime_for(hwnd) {
                runtime.state.settings = settings::load();
                let display = DisplayOptions::from(&runtime.state.settings);
                runtime
                    .overlay
                    .set_state(runtime.state.state.clone(), display);
                runtime.sync_overlay();
                runtime.request_refresh();
            }
            LRESULT(0)
        }
        WM_APP_REVIEW_CLICKED => {
            if let Some(runtime) = runtime_for(hwnd) {
                runtime.start_uncommitted_review();
            }
            LRESULT(0)
        }
        WM_APP_PLAN_CLICKED => {
            if let Some(runtime) = runtime_for(hwnd) {
                runtime.start_plan();
            }
            LRESULT(0)
        }
        WM_APP_PROMPT_CLICKED => {
            if let Some(runtime) = runtime_for(hwnd) {
                runtime.start_prompt_menu();
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            if let Some(runtime) = runtime_for(hwnd) {
                match wparam.0 & 0xffff {
                    IDM_REFRESH => runtime.request_refresh(),
                    IDM_TOGGLE_OVERLAY => runtime.toggle_overlay(),
                    IDM_SETTINGS => settings_window::open(hwnd),
                    IDM_DIAGNOSE => {
                        let _ = std::thread::spawn(crate::diagnose::run);
                    }
                    IDM_EXIT => {
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Some(mut runtime) = take_runtime(hwnd) {
                runtime.shutdown();
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe fn runtime_for(hwnd: HWND) -> Option<&'static mut AppRuntime> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AppRuntime).as_mut()
}

unsafe fn take_runtime(hwnd: HWND) -> Option<Box<AppRuntime>> {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AppRuntime;
    if pointer.is_null() {
        None
    } else {
        let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        Some(Box::from_raw(pointer))
    }
}
