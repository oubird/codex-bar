//! Native Win32 settings window.

#![cfg(windows)]

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Controls::BST_CHECKED;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, GetWindowTextW,
    RegisterClassExW, SendMessageW, SetWindowLongPtrW, ShowWindow, BM_GETCHECK, BM_SETCHECK,
    BS_AUTOCHECKBOX, BS_AUTORADIOBUTTON, BS_PUSHBUTTON, CW_USEDEFAULT, ES_AUTOHSCROLL,
    GWLP_USERDATA, HMENU, SW_SHOW, WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_NCCREATE,
    WNDCLASSEXW, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

use crate::settings::{self, ProxyMode, SettingsPatch};

pub const WM_APP_SETTINGS_CHANGED: u32 = 0x8000 + 41;
const CLASS: &str = "CodexPlusSettingsWindow";
const ID_OVERLAY: usize = 101;
const ID_INTERVAL: usize = 102;
const ID_PROXY_ENV: usize = 201;
const ID_PROXY_DIRECT: usize = 202;
const ID_PROXY_CUSTOM: usize = 203;
const ID_PROXY_ADDRESS: usize = 204;
const ID_SAVE: usize = 301;
const ID_CANCEL: usize = 302;

struct SettingsWindowData {
    host: HWND,
}

pub fn open(host: HWND) {
    unsafe {
        let class = wide(CLASS);
        let definition = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(proc),
            hInstance: HINSTANCE::default(),
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassExW(&definition);
        let data = Box::into_raw(Box::new(SettingsWindowData { host }));
        let title = wide("CodexPlus Settings");
        let hwnd = CreateWindowExW(
            Default::default(),
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            470,
            330,
            host,
            None,
            HINSTANCE::default(),
            Some(data.cast()),
        );
        match hwnd {
            Ok(hwnd) => {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
            Err(_) => {
                let _ = Box::from_raw(data);
            }
        }
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            let create = lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
            if !create.is_null() {
                let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
            }
            LRESULT(1)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_CREATE => {
            build_controls(hwnd);
            LRESULT(0)
        }
        WM_COMMAND => {
            match wparam.0 & 0xffff {
                ID_SAVE => save(hwnd),
                ID_CANCEL => {
                    let _ = DestroyWindow(hwnd);
                }
                ID_PROXY_ENV | ID_PROXY_DIRECT | ID_PROXY_CUSTOM => {
                    select_proxy_mode(hwnd, wparam.0 & 0xffff);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsWindowData;
            if !pointer.is_null() {
                let _ = Box::from_raw(pointer);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn build_controls(hwnd: HWND) {
    let current = settings::load();
    label(hwnd, "Usage overlay", 20, 20, 160, 22);
    button(
        hwnd,
        "Show Codex title-bar overlay",
        BS_AUTOCHECKBOX as u32,
        ID_OVERLAY,
        20,
        45,
        250,
        25,
    );
    check(hwnd, ID_OVERLAY, current.overlay_enabled);
    label(
        hwnd,
        "Refresh interval (seconds, 60-3600):",
        20,
        78,
        260,
        22,
    );
    edit(
        hwnd,
        ID_INTERVAL,
        &current.refresh_interval_secs.to_string(),
        285,
        75,
        110,
        24,
        true,
    );

    label(hwnd, "HTTP proxy", 20, 118, 160, 22);
    button(
        hwnd,
        "Use environment proxy",
        BS_AUTORADIOBUTTON as u32,
        ID_PROXY_ENV,
        20,
        143,
        260,
        23,
    );
    button(
        hwnd,
        "Force direct connection",
        BS_AUTORADIOBUTTON as u32,
        ID_PROXY_DIRECT,
        20,
        166,
        260,
        23,
    );
    button(
        hwnd,
        "Use custom proxy address",
        BS_AUTORADIOBUTTON as u32,
        ID_PROXY_CUSTOM,
        20,
        189,
        260,
        23,
    );
    match current.proxy_mode {
        ProxyMode::Environment => check(hwnd, ID_PROXY_ENV, true),
        ProxyMode::Direct => check(hwnd, ID_PROXY_DIRECT, true),
        ProxyMode::Custom => check(hwnd, ID_PROXY_CUSTOM, true),
    }
    label(
        hwnd,
        "Custom address (not shown again after save):",
        40,
        214,
        320,
        20,
    );
    edit(hwnd, ID_PROXY_ADDRESS, "", 40, 235, 350, 24, false);
    button(
        hwnd,
        "Save",
        BS_PUSHBUTTON as u32,
        ID_SAVE,
        245,
        270,
        70,
        28,
    );
    button(
        hwnd,
        "Cancel",
        BS_PUSHBUTTON as u32,
        ID_CANCEL,
        325,
        270,
        70,
        28,
    );
    update_custom_enabled(hwnd);
}

unsafe fn save(hwnd: HWND) {
    let mode = if checked(hwnd, ID_PROXY_DIRECT) {
        ProxyMode::Direct
    } else if checked(hwnd, ID_PROXY_CUSTOM) {
        ProxyMode::Custom
    } else {
        ProxyMode::Environment
    };
    let mut settings_value = settings::load();
    if mode == ProxyMode::Custom {
        let address = text(hwnd, ID_PROXY_ADDRESS);
        if !address.trim().is_empty() && settings::save_custom_proxy_url(address.trim()).is_err() {
            show_invalid_proxy_message(hwnd);
            return;
        }
        if address.trim().is_empty() && settings_value.custom_proxy_credential_target.is_none() {
            show_invalid_proxy_message(hwnd);
            return;
        }
    }
    let interval = text(hwnd, ID_INTERVAL).parse::<u32>().unwrap_or(300);
    SettingsPatch {
        overlay_enabled: Some(checked(hwnd, ID_OVERLAY)),
        refresh_interval_secs: Some(interval),
        proxy_mode: Some(mode),
        custom_proxy_configured: (mode == ProxyMode::Custom).then_some(true),
    }
    .apply_to(&mut settings_value);
    if settings::save(&settings_value).is_err() {
        return;
    }
    let data = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsWindowData;
    if let Some(data) = data.as_ref() {
        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
            data.host,
            WM_APP_SETTINGS_CHANGED,
            WPARAM(0),
            LPARAM(0),
        );
    }
    let _ = DestroyWindow(hwnd);
}

unsafe fn select_proxy_mode(hwnd: HWND, selected: usize) {
    for id in [ID_PROXY_ENV, ID_PROXY_DIRECT, ID_PROXY_CUSTOM] {
        check(hwnd, id, id == selected);
    }
    update_custom_enabled(hwnd);
}

unsafe fn update_custom_enabled(hwnd: HWND) {
    let edit = child(hwnd, ID_PROXY_ADDRESS);
    let _ = EnableWindow(edit, checked(hwnd, ID_PROXY_CUSTOM));
}

unsafe fn show_invalid_proxy_message(hwnd: HWND) {
    let message = wide("Enter a valid http:// or https:// custom proxy address.");
    let title = wide("CodexPlus Settings");
    let _ = windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
        hwnd,
        PCWSTR(message.as_ptr()),
        PCWSTR(title.as_ptr()),
        Default::default(),
    );
}

unsafe fn label(hwnd: HWND, text_value: &str, x: i32, y: i32, width: i32, height: i32) {
    let class = wide("STATIC");
    let text_value = wide(text_value);
    let _ = CreateWindowExW(
        Default::default(),
        PCWSTR(class.as_ptr()),
        PCWSTR(text_value.as_ptr()),
        WS_CHILD | WS_VISIBLE,
        x,
        y,
        width,
        height,
        hwnd,
        None,
        HINSTANCE::default(),
        None,
    );
}
#[allow(clippy::too_many_arguments)]
unsafe fn button(
    hwnd: HWND,
    text_value: &str,
    style: u32,
    id: usize,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let class = wide("BUTTON");
    let text_value = wide(text_value);
    let _ = CreateWindowExW(
        Default::default(),
        PCWSTR(class.as_ptr()),
        PCWSTR(text_value.as_ptr()),
        WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | style),
        x,
        y,
        width,
        height,
        hwnd,
        HMENU(id as *mut _),
        HINSTANCE::default(),
        None,
    );
}
#[allow(clippy::too_many_arguments)]
unsafe fn edit(
    hwnd: HWND,
    id: usize,
    value: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    digits: bool,
) {
    let class = wide("EDIT");
    let value = wide(value);
    let style = WS_CHILD.0
        | WS_VISIBLE.0
        | ES_AUTOHSCROLL as u32
        | if digits {
            windows::Win32::UI::WindowsAndMessaging::ES_NUMBER as u32
        } else {
            0
        };
    let _ = CreateWindowExW(
        Default::default(),
        PCWSTR(class.as_ptr()),
        PCWSTR(value.as_ptr()),
        WINDOW_STYLE(style),
        x,
        y,
        width,
        height,
        hwnd,
        HMENU(id as *mut _),
        HINSTANCE::default(),
        None,
    );
}
unsafe fn child(hwnd: HWND, id: usize) -> HWND {
    windows::Win32::UI::WindowsAndMessaging::GetDlgItem(hwnd, id as i32).unwrap_or_default()
}
unsafe fn check(hwnd: HWND, id: usize, value: bool) {
    let _ = SendMessageW(
        child(hwnd, id),
        BM_SETCHECK,
        WPARAM(if value { BST_CHECKED.0 as usize } else { 0 }),
        LPARAM(0),
    );
}
unsafe fn checked(hwnd: HWND, id: usize) -> bool {
    SendMessageW(child(hwnd, id), BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == BST_CHECKED.0 as isize
}
unsafe fn text(hwnd: HWND, id: usize) -> String {
    let mut buf = [0u16; 1024];
    let len = GetWindowTextW(child(hwnd, id), &mut buf);
    String::from_utf16_lossy(&buf[..len as usize])
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
