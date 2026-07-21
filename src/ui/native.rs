//! Win32 helpers shared between overlay, target-window tracker, and tray.
//!
//! The MVP scaffolding exposes a *minimal* Win32 API surface so that
//! `cargo check`/`cargo test` work on every platform. The larger Win32
//! plumbing — host message pump, `SetWinEventHook`, tray icon, layered
//! paint — is filled in during the end-to-end pass once the underlying
//! logic has stabilised.

#![cfg(windows)]

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetAncestor, GetClassNameW, GetForegroundWindow, GetWindowRect,
    GetWindowThreadProcessId, IsWindowVisible, GA_ROOT,
};

pub fn wide_str(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn pcwstr_to_string(value: PCWSTR) -> String {
    unsafe {
        let mut len = 0usize;
        let mut cur = value.0;
        while !cur.is_null() && *cur != 0 {
            len += 1;
            cur = cur.add(1);
        }
        let slice = std::slice::from_raw_parts(value.0, len);
        String::from_utf16_lossy(slice)
    }
}

pub fn overlay_extended_styles_bits() -> u32 {
    // Mirrors the production combination used in the overlay:
    // WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED
    // | WS_EX_TRANSPARENT | WS_EX_TOPMOST.
    0x00000080 | 0x08000000 | 0x00080000 | 0x00000020 | 0x00000008
}

pub fn window_rect(hwnd: HWND) -> Option<RECT> {
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_ok() {
            Some(rect)
        } else {
            None
        }
    }
}

pub fn is_window_visible(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd).as_bool() }
}

pub fn class_name(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 256];
        let len = GetClassNameW(hwnd, &mut buf);
        if len == 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

pub fn root_owner(hwnd: HWND) -> HWND {
    unsafe { GetAncestor(hwnd, GA_ROOT) }
}

pub fn foreground_window() -> HWND {
    unsafe { GetForegroundWindow() }
}

pub fn window_thread_process_id(hwnd: HWND) -> (u32, u32) {
    let mut pid = 0u32;
    let tid = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    (tid, pid)
}

pub fn enumerate_top_level() -> Vec<HWND> {
    struct State(Vec<HWND>);
    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> windows::Win32::Foundation::BOOL {
        let state = &mut *(lparam.0 as *mut State);
        if !hwnd.0.is_null() {
            state.0.push(hwnd);
        }
        windows::Win32::Foundation::BOOL(1)
    }
    let mut state = State(Vec::new());
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(&mut state as *mut _ as isize));
    }
    state.0
}

/// Run the application.
///
/// In the MVP scaffolding the loop performs the initial poll kickoff and
/// then returns. The full Win32 host window, overlay host, and tray icon
/// are added during the Windows-only end-to-end pass.
pub fn run_app() -> Result<(), String> {
    let (controller, _rx) = crate::ui::poll::start();
    controller.request_refresh();
    Ok(())
}
