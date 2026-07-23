//! Hidden Win32 host window and resident application lifecycle.

#![cfg(windows)]

use std::sync::mpsc::Receiver;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowLongPtrW, PostQuitMessage, RegisterClassExW, SetTimer, SetWindowLongPtrW, ShowWindow,
    TranslateMessage, GWLP_USERDATA, MSG, SW_HIDE, WINDOW_STYLE, WM_COMMAND, WM_DESTROY, WM_TIMER,
    WNDCLASSEXW,
};

use super::app_state::{AppState, RefreshDisposition};
use super::overlay::OverlayHandle;
use super::poll::{self, PollOutcome, PollSender};
use super::settings::{self as settings_window, WM_APP_SETTINGS_CHANGED};
use super::target_window;
use super::tray::{
    TrayIcon, IDM_DIAGNOSE, IDM_EXIT, IDM_REFRESH, IDM_SETTINGS, IDM_TOGGLE_OVERLAY, WM_APP_TRAY,
};
use crate::settings::{self, SettingsPatch};

const HOST_CLASS: &str = "CodexPlusHostWindow";
const TIMER_RECONCILE: usize = 1;
const TIMER_POLL: usize = 2;
const TIMER_COUNTDOWN: usize = 3;

pub fn wide_str(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn overlay_extended_styles_bits() -> u32 {
    0x00000080 | 0x08000000 | 0x00080000 | 0x00000020 | 0x00000008
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
    tray: TrayIcon,
    poll_sender: PollSender,
    poll_receiver: Receiver<PollOutcome>,
}

impl AppRuntime {
    fn new(hwnd: HWND) -> Result<Self, String> {
        let settings = settings::load();
        let cached = crate::storage::load();
        let state = AppState::new(settings, cached);
        let overlay = OverlayHandle::create()?;
        let mut tray = TrayIcon::new(hwnd);
        tray.add_or_update(&state.state);
        let (poll_sender, poll_receiver) = poll::start();
        let mut runtime = Self {
            state,
            overlay,
            tray,
            poll_sender,
            poll_receiver,
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
            self.overlay.set_state(self.state.state.clone());
            self.tray.add_or_update(&self.state.state);
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
        let target =
            target_window::foreground_codex_window().map(|identity| HWND(identity.hwnd as *mut _));
        self.overlay
            .sync_to_target(target, self.state.settings.overlay_enabled);
        self.tray.add_or_update(&self.state.state);
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
                        runtime.overlay.set_state(runtime.state.state.clone());
                        runtime.tray.add_or_update(&runtime.state.state);
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
                    runtime
                        .tray
                        .show_menu(runtime.state.settings.overlay_enabled, &runtime.state.state);
                }
            }
            LRESULT(0)
        }
        WM_APP_SETTINGS_CHANGED => {
            if let Some(runtime) = runtime_for(hwnd) {
                runtime.state.settings = settings::load();
                runtime.sync_overlay();
                runtime.request_refresh();
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
