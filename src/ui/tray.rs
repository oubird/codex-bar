//! Windows notification-area icon and native context menu.

#![cfg(windows)]

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, SetForegroundWindow, TrackPopupMenu,
    MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD,
    TPM_RIGHTBUTTON,
};

use super::native;
use crate::formatting::{format_tray_tooltip, DisplayOptions};
use crate::model::{UsageSnapshot, ViewState};

pub const WM_APP_TRAY: u32 = 0x8000 + 30;
pub const IDM_REFRESH: usize = 1001;
pub const IDM_TOGGLE_OVERLAY: usize = 1002;
pub const IDM_SETTINGS: usize = 1003;
pub const IDM_DIAGNOSE: usize = 1004;
pub const IDM_EXIT: usize = 1005;
const TRAY_ICON_ID: u32 = 1;

pub struct TrayIcon {
    hwnd: HWND,
    added: bool,
}

impl TrayIcon {
    pub fn new(hwnd: HWND) -> Self {
        Self { hwnd, added: false }
    }

    pub fn add_or_update(&mut self, state: &ViewState, display: DisplayOptions) {
        unsafe {
            let mut data = tray_data(self.hwnd);
            data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            data.uCallbackMessage = WM_APP_TRAY;
            data.hIcon = native::load_app_icon();
            let now = UsageSnapshot::now();
            let shift = native::local_shift_seconds(now);
            write_tip(&mut data, &format_tray_tooltip(state, now, display, shift));
            let operation = if self.added { NIM_MODIFY } else { NIM_ADD };
            if Shell_NotifyIconW(operation, &data).as_bool() {
                self.added = true;
            }
        }
    }

    pub fn remove(&mut self) {
        if !self.added {
            return;
        }
        unsafe {
            let data = tray_data(self.hwnd);
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
        self.added = false;
    }

    pub fn show_menu(&self, overlay_enabled: bool, state: &ViewState, display: DisplayOptions) {
        unsafe {
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            let now = UsageSnapshot::now();
            let shift = native::local_shift_seconds(now);
            let tooltip = format_tray_tooltip(state, now, display, shift);
            let summary = tooltip.lines().nth(1).unwrap_or("正在加载 Codex 用量…");
            let summary_wide = wide(summary);
            let refresh_wide = wide("立即刷新");
            let overlay_wide = wide("显示标题栏用量");
            let settings_wide = wide("设置…");
            let diagnose_wide = wide("运行诊断");
            let exit_wide = wide("退出");
            let _ = AppendMenuW(
                menu,
                MF_STRING | MF_GRAYED,
                0,
                PCWSTR(summary_wide.as_ptr()),
            );
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, IDM_REFRESH, PCWSTR(refresh_wide.as_ptr()));
            let flag = if overlay_enabled {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            };
            let _ = AppendMenuW(
                menu,
                flag,
                IDM_TOGGLE_OVERLAY,
                PCWSTR(overlay_wide.as_ptr()),
            );
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                IDM_SETTINGS,
                PCWSTR(settings_wide.as_ptr()),
            );
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                IDM_DIAGNOSE,
                PCWSTR(diagnose_wide.as_ptr()),
            );
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, IDM_EXIT, PCWSTR(exit_wide.as_ptr()));
            let mut point = POINT::default();
            let _ = GetCursorPos(&mut point);
            let _ = SetForegroundWindow(self.hwnd);
            let selected = TrackPopupMenu(
                menu,
                TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD,
                point.x,
                point.y,
                0,
                self.hwnd,
                None,
            );
            if selected.0 != 0 {
                let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    self.hwnd,
                    windows::Win32::UI::WindowsAndMessaging::WM_COMMAND,
                    windows::Win32::Foundation::WPARAM(selected.0 as usize),
                    windows::Win32::Foundation::LPARAM(0),
                );
            }
            let _ = DestroyMenu(menu);
        }
    }
}

unsafe fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        ..Default::default()
    }
}

fn write_tip(data: &mut NOTIFYICONDATAW, text: &str) {
    let mut index = 0;
    for unit in text.encode_utf16() {
        if index + 1 >= data.szTip.len() {
            break;
        }
        data.szTip[index] = unit;
        index += 1;
    }
    data.szTip[index] = 0;
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
