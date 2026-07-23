//! Native Win32 settings window.

#![cfg(windows)]

use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DeleteObject, FillRect, FrameRect, HBRUSH, HDC, HFONT,
    InvalidateRect, SetBkMode, SetTextColor, DEFAULT_CHARSET, FW_NORMAL, TRANSPARENT,
};
use windows::Win32::UI::Controls::BST_CHECKED;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetWindowLongPtrW,
    GetWindowTextW, PostMessageW, RegisterClassExW, SendMessageW, SetWindowLongPtrW, ShowWindow,
    BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_AUTORADIOBUTTON, BS_PUSHBUTTON, CW_USEDEFAULT,
    ES_AUTOHSCROLL, GWL_STYLE, GWLP_USERDATA, HMENU, SW_SHOW, WINDOW_STYLE, WM_CLOSE, WM_COMMAND,
    WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY, WM_NCCREATE, WM_PAINT, WNDCLASSEXW, WS_CAPTION,
    WS_CHILD, WS_CLIPCHILDREN, WS_DISABLED, WS_MINIMIZEBOX, WS_SYSMENU, WS_VISIBLE,
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

// Colors are stored in 0x00BBGGRR layout (Win32 COLORREF).
const COLOR_WINDOW_BG: u32 = 0x00F4F5F7; // soft gray-white
const COLOR_CARD_BG: u32 = 0x00FFFFFF; // white card
const COLOR_DISABLED_BG: u32 = 0x00ECEEF1;
const COLOR_BORDER: u32 = 0x00E1E5EA;
const COLOR_TEXT: u32 = 0x00202226;

struct SettingsWindowData {
    host: HWND,
    font: HFONT,
    font_bold: HFONT,
    brush_window: HBRUSH,
    brush_card: HBRUSH,
    brush_border: HBRUSH,
    brush_disabled: HBRUSH,
}

impl SettingsWindowData {
    unsafe fn create(host: HWND, dpi: u32) -> Self {
        Self {
            host,
            font: create_ui_font(dpi, false),
            font_bold: create_ui_font(dpi, true),
            brush_window: CreateSolidBrush(COLORREF(COLOR_WINDOW_BG)),
            brush_card: CreateSolidBrush(COLORREF(COLOR_CARD_BG)),
            brush_border: CreateSolidBrush(COLORREF(COLOR_BORDER)),
            brush_disabled: CreateSolidBrush(COLORREF(COLOR_DISABLED_BG)),
        }
    }

    unsafe fn dispose(&mut self) {
        if !self.font.is_invalid() {
            let _ = DeleteObject(self.font);
        }
        if !self.font_bold.is_invalid() {
            let _ = DeleteObject(self.font_bold);
        }
        if !self.brush_window.is_invalid() {
            let _ = DeleteObject(self.brush_window);
        }
        if !self.brush_card.is_invalid() {
            let _ = DeleteObject(self.brush_card);
        }
        if !self.brush_border.is_invalid() {
            let _ = DeleteObject(self.brush_border);
        }
        if !self.brush_disabled.is_invalid() {
            let _ = DeleteObject(self.brush_disabled);
        }
    }
}

unsafe fn create_ui_font(dpi: u32, bold: bool) -> HFONT {
    let face: Vec<u16> = "Microsoft YaHei UI"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let height_px = ((9 * dpi + 24) / 48) as i32;
    let weight: u32 = if bold { FW_NORMAL.0 + 200 } else { FW_NORMAL.0 };
    CreateFontW(
        -height_px,
        0,
        0,
        0,
        weight as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        0,
        0,
        0,
        0,
        PCWSTR(face.as_ptr()),
    )
}

unsafe fn is_window_disabled(hwnd: HWND) -> bool {
    let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
    (style & (WS_DISABLED.0 as isize)) != 0
}

fn hdc_from_wparam(wparam: WPARAM) -> HDC {
    HDC(wparam.0 as *mut _)
}

fn hwnd_from_lparam(lparam: LPARAM) -> HWND {
    HWND(lparam.0 as *mut _)
}

unsafe fn get_data(hwnd: HWND) -> Option<&'static mut SettingsWindowData> {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsWindowData;
    pointer.as_mut()
}

pub fn open(host: HWND) {
    unsafe {
        let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(host).max(96);
        let class = wide(CLASS);
        let definition = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(proc),
            hInstance: HINSTANCE::default(),
            lpszClassName: PCWSTR(class.as_ptr()),
            hbrBackground: CreateSolidBrush(COLORREF(COLOR_WINDOW_BG)),
            ..Default::default()
        };
        let _ = RegisterClassExW(&definition);
        let data = Box::into_raw(Box::new(SettingsWindowData::create(host, dpi)));
        let title = wide("CodexPlus 设置");
        let hwnd = CreateWindowExW(
            Default::default(),
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_VISIBLE | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            scale(540, dpi),
            scale(540, dpi),
            host,
            None,
            HINSTANCE::default(),
            Some(data.cast()),
        );
        match hwnd {
            Ok(hwnd) => {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = InvalidateRect(hwnd, None, true);
            }
            Err(_) => {
                let mut data = Box::from_raw(data);
                data.dispose();
            }
        }
    }
}

fn scale(value: i32, dpi: u32) -> i32 {
    (value * dpi as i32 + 48) / 96
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
        WM_CTLCOLOREDIT => {
            let hdc = hdc_from_wparam(wparam);
            let edit = hwnd_from_lparam(lparam);
            paint_edit_background(hwnd, hdc, edit);
            let disabled = is_window_disabled(edit);
            LRESULT(get_brush_for_edit(hwnd, disabled).0 as isize)
        }
        WM_CTLCOLORSTATIC => {
            let hdc = hdc_from_wparam(wparam);
            paint_static_background(hwnd, hdc);
            LRESULT(get_data(hwnd)
                .map(|d| d.brush_card.0 as isize)
                .unwrap_or(0))
        }
        WM_PAINT => {
            paint_chrome(hwnd);
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsWindowData;
            if !pointer.is_null() {
                let mut data = Box::from_raw(pointer);
                data.dispose();
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn paint_edit_background(hwnd: HWND, hdc: HDC, edit: HWND) {
    let disabled = is_window_disabled(edit);
    let Some(data) = get_data(hwnd) else {
        return;
    };
    let brush = if disabled {
        data.brush_disabled
    } else {
        data.brush_card
    };
    let mut rect = RECT::default();
    let _ = GetClientRect(edit, &mut rect);
    rect.left -= 1;
    rect.top -= 1;
    rect.right += 1;
    rect.bottom += 1;
    let _ = FillRect(hdc, &rect, brush);
    let _ = FrameRect(hdc, &rect, data.brush_border);
    let _ = SetBkMode(hdc, TRANSPARENT);
    let _ = SetTextColor(hdc, COLORREF(COLOR_TEXT));
}

unsafe fn get_brush_for_edit(hwnd: HWND, disabled: bool) -> HBRUSH {
    match get_data(hwnd) {
        Some(data) if disabled => data.brush_disabled,
        Some(data) => data.brush_card,
        None => HBRUSH::default(),
    }
}

unsafe fn paint_static_background(hwnd: HWND, hdc: HDC) {
    let _ = SetBkMode(hdc, TRANSPARENT);
    let _ = SetTextColor(hdc, COLORREF(COLOR_TEXT));
    let _ = get_data(hwnd);
}

unsafe fn paint_chrome(hwnd: HWND) {
    use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};

    let mut paint = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut paint);
    let Some(data) = get_data(hwnd) else {
        let _ = EndPaint(hwnd, &paint);
        return;
    };
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let _ = FillRect(hdc, &client, data.brush_window);

    // Two cards with simple rounded feel via FrameRect.
    let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96);
    let card_padding = scale(20, dpi);
    let card_top = scale(56, dpi);
    let card_left = client.left + card_padding;
    let card_right = client.right - card_padding;
    let display_card = RECT {
        left: card_left,
        top: card_top,
        right: card_right,
        bottom: card_top + scale(168, dpi),
    };
    let proxy_card = RECT {
        left: card_left,
        top: display_card.bottom + scale(16, dpi),
        right: card_right,
        bottom: display_card.bottom + scale(16, dpi) + scale(248, dpi),
    };

    draw_card(hdc, data, display_card);
    draw_card(hdc, data, proxy_card);

    let _ = EndPaint(hwnd, &paint);
}

unsafe fn draw_card(hdc: HDC, data: &SettingsWindowData, rect: RECT) {
    let _ = FillRect(hdc, &rect, data.brush_card);
    let _ = FrameRect(hdc, &rect, data.brush_border);
}

unsafe fn build_controls(hwnd: HWND) {
    let Some(data) = get_data(hwnd) else {
        return;
    };
    let current = settings::load();
    let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96);

    // Card metrics
    let card_padding = scale(20, dpi);
    let card_top = scale(56, dpi);
    let card_left = card_padding;
    let card_right = scale(540, dpi) - card_padding;
    let display_card_inner_top = card_top + scale(44, dpi);
    let display_card_height = scale(168, dpi);
    let proxy_card_top = card_top + display_card_height + scale(16, dpi) + scale(44, dpi);
    let proxy_card_height = scale(248, dpi);
    let bar_top =
        card_top + display_card_height + scale(16, dpi) + proxy_card_height + scale(16, dpi);

    // ---- Display settings card ----
    button(
        hwnd,
        data.font,
        "在 Codex 标题栏显示用量",
        BS_AUTOCHECKBOX as u32,
        ID_OVERLAY,
        card_left + scale(18, dpi),
        display_card_inner_top,
        card_right - card_left - scale(36, dpi),
        scale(22, dpi),
    );
    check(hwnd, ID_OVERLAY, current.overlay_enabled);

    label(
        hwnd,
        data.font,
        "刷新间隔（秒，60–3600）",
        card_left + scale(18, dpi),
        display_card_inner_top + scale(56, dpi),
        scale(280, dpi),
        scale(22, dpi),
    );
    edit(
        hwnd,
        data.font,
        ID_INTERVAL,
        &current.refresh_interval_secs.to_string(),
        card_left + scale(18, dpi) + scale(290, dpi),
        display_card_inner_top + scale(52, dpi),
        scale(170, dpi),
        scale(28, dpi),
        true,
    );

    // ---- Proxy card ----
    button(
        hwnd,
        data.font,
        "使用系统环境代理",
        BS_AUTORADIOBUTTON as u32,
        ID_PROXY_ENV,
        card_left + scale(18, dpi),
        proxy_card_top,
        card_right - card_left - scale(36, dpi),
        scale(22, dpi),
    );
    button(
        hwnd,
        data.font,
        "强制直连（忽略所有代理）",
        BS_AUTORADIOBUTTON as u32,
        ID_PROXY_DIRECT,
        card_left + scale(18, dpi),
        proxy_card_top + scale(30, dpi),
        card_right - card_left - scale(36, dpi),
        scale(22, dpi),
    );
    button(
        hwnd,
        data.font,
        "使用自定义代理地址",
        BS_AUTORADIOBUTTON as u32,
        ID_PROXY_CUSTOM,
        card_left + scale(18, dpi),
        proxy_card_top + scale(60, dpi),
        card_right - card_left - scale(36, dpi),
        scale(22, dpi),
    );
    match current.proxy_mode {
        ProxyMode::Environment => check(hwnd, ID_PROXY_ENV, true),
        ProxyMode::Direct => check(hwnd, ID_PROXY_DIRECT, true),
        ProxyMode::Custom => check(hwnd, ID_PROXY_CUSTOM, true),
    }

    label(
        hwnd,
        data.font,
        "代理地址（保存后不再回显，留空表示继续使用已保存地址）",
        card_left + scale(18, dpi),
        proxy_card_top + scale(92, dpi),
        card_right - card_left - scale(36, dpi),
        scale(36, dpi),
    );
    edit(
        hwnd,
        data.font,
        ID_PROXY_ADDRESS,
        "",
        card_left + scale(18, dpi),
        proxy_card_top + scale(132, dpi),
        card_right - card_left - scale(36, dpi),
        scale(32, dpi),
        false,
    );

    // ---- Action bar ----
    button(
        hwnd,
        data.font,
        "取消",
        BS_PUSHBUTTON as u32,
        ID_CANCEL,
        card_right - scale(96, dpi) - scale(96, dpi),
        bar_top,
        scale(88, dpi),
        scale(34, dpi),
    );
    button(
        hwnd,
        data.font,
        "保存",
        BS_PUSHBUTTON as u32,
        ID_SAVE,
        card_right - scale(88, dpi),
        bar_top,
        scale(88, dpi),
        scale(34, dpi),
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
        let _ = PostMessageW(data.host, WM_APP_SETTINGS_CHANGED, WPARAM(0), LPARAM(0));
    }
    let _ = DestroyWindow(hwnd);
}

unsafe fn select_proxy_mode(hwnd: HWND, selected: usize) {
    for id in [ID_PROXY_ENV, ID_PROXY_DIRECT, ID_PROXY_CUSTOM] {
        check(hwnd, id, id == selected);
    }
    update_custom_enabled(hwnd);
    let _ = InvalidateRect(hwnd, None, true);
}

unsafe fn update_custom_enabled(hwnd: HWND) {
    let edit = child(hwnd, ID_PROXY_ADDRESS);
    let _ = EnableWindow(edit, checked(hwnd, ID_PROXY_CUSTOM));
    let _ = InvalidateRect(edit, None, true);
}

unsafe fn show_invalid_proxy_message(hwnd: HWND) {
    let message = wide("请输入有效的 http:// 或 https:// 自定义代理地址。");
    let title = wide("CodexPlus 设置");
    let _ = windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
        hwnd,
        PCWSTR(message.as_ptr()),
        PCWSTR(title.as_ptr()),
        Default::default(),
    );
}

unsafe fn label(
    hwnd: HWND,
    font: HFONT,
    text_value: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let class = wide("STATIC");
    let text_value = wide(text_value);
    let created = CreateWindowExW(
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
    if let Ok(h) = created {
        let _ = SendMessageW(
            h,
            windows::Win32::UI::WindowsAndMessaging::WM_SETFONT,
            WPARAM(font.0 as usize),
            LPARAM(1),
        );
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn button(
    hwnd: HWND,
    font: HFONT,
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
    let created = CreateWindowExW(
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
    if let Ok(h) = created {
        let _ = SendMessageW(
            h,
            windows::Win32::UI::WindowsAndMessaging::WM_SETFONT,
            WPARAM(font.0 as usize),
            LPARAM(1),
        );
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn edit(
    hwnd: HWND,
    font: HFONT,
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
    let created = CreateWindowExW(
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
    if let Ok(h) = created {
        let _ = SendMessageW(
            h,
            windows::Win32::UI::WindowsAndMessaging::WM_SETFONT,
            WPARAM(font.0 as usize),
            LPARAM(1),
        );
    }
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
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
