//! Native Win32 settings window.

#![cfg(windows)]

use std::sync::atomic::{AtomicIsize, Ordering};

use super::native;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, EndPaint, FillRect, FrameRect,
    InvalidateRect, SetBkMode, SetTextColor, DEFAULT_CHARSET, FW_NORMAL, HBRUSH, HDC, HFONT,
    PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::UI::Controls::{
    InitCommonControlsEx, SetWindowTheme, ICC_STANDARD_CLASSES, INITCOMMONCONTROLSEX,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetDlgCtrlID, GetWindowLongPtrW,
    GetWindowTextW, IsDialogMessageW, IsWindow, PostMessageW, RegisterClassExW, SendMessageW,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, BM_GETCHECK, BM_SETCHECK,
    BS_AUTOCHECKBOX, BS_AUTORADIOBUTTON, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CW_USEDEFAULT,
    DM_SETDEFID, ES_AUTOHSCROLL, GWLP_USERDATA, GWL_STYLE, HMENU, MSG, SWP_NOACTIVATE,
    SWP_NOZORDER, SW_SHOW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_CREATE,
    WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND,
    WM_NCCREATE, WM_PAINT, WNDCLASSEXW, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN, WS_DISABLED,
    WS_EX_CLIENTEDGE, WS_GROUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};

use crate::settings::{self, ProxyMode, ResetTimeDisplay, SettingsPatch, UsageDisplay};

pub const WM_APP_SETTINGS_CHANGED: u32 = 0x8000 + 41;

const CLASS: &str = "CodexPlusSettingsWindow";

const ID_SAVE: usize = 1;
const ID_CANCEL: usize = 2;
const ID_PAGE_TITLE: usize = 10;
const ID_PAGE_SUBTITLE: usize = 11;
const ID_DISPLAY_TITLE: usize = 12;
const ID_PROXY_TITLE: usize = 13;
const ID_PROXY_HELP: usize = 14;
const ID_OVERLAY: usize = 101;
const ID_INTERVAL_LABEL: usize = 102;
const ID_INTERVAL: usize = 103;
const ID_USAGE_LABEL: usize = 104;
const ID_USAGE_REMAINING: usize = 105;
const ID_USAGE_USED: usize = 106;
const ID_RESET_LABEL: usize = 107;
const ID_RESET_COUNTDOWN: usize = 108;
const ID_RESET_ABSOLUTE: usize = 109;
const ID_PROXY_ENV: usize = 201;
const ID_PROXY_DIRECT: usize = 202;
const ID_PROXY_CUSTOM: usize = 203;
const ID_PROXY_ADDRESS_LABEL: usize = 204;
const ID_PROXY_ADDRESS: usize = 205;

const CLIENT_WIDTH: i32 = 560;
const CLIENT_HEIGHT: i32 = 678;

// Colors are stored in 0x00BBGGRR layout (Win32 COLORREF).
const COLOR_WINDOW_BG: u32 = 0x00F4F5F7;
const COLOR_CARD_BG: u32 = 0x00FFFFFF;
const COLOR_DISABLED_BG: u32 = 0x00ECEEF1;
const COLOR_BORDER: u32 = 0x00E1E5EA;
const COLOR_TEXT: u32 = 0x00202226;
const COLOR_TEXT_SECONDARY: u32 = 0x00746F6A;

static SETTINGS_WINDOW: AtomicIsize = AtomicIsize::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LogicalRect {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}

impl LogicalRect {
    const fn new(left: i32, top: i32, width: i32, height: i32) -> Self {
        Self {
            left,
            top,
            width,
            height,
        }
    }

    const fn right(self) -> i32 {
        self.left + self.width
    }

    const fn bottom(self) -> i32 {
        self.top + self.height
    }
}

#[derive(Clone, Copy, Debug)]
struct SettingsLayout {
    display_card: LogicalRect,
    proxy_card: LogicalRect,
    save_button: LogicalRect,
    cancel_button: LogicalRect,
}

impl SettingsLayout {
    const fn new() -> Self {
        Self {
            display_card: LogicalRect::new(20, 88, 520, 240),
            proxy_card: LogicalRect::new(20, 344, 520, 244),
            cancel_button: LogicalRect::new(350, 618, 88, 36),
            save_button: LogicalRect::new(448, 618, 92, 36),
        }
    }
}

struct SettingsWindowData {
    host: HWND,
    dpi: u32,
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
            dpi,
            font: create_ui_font(dpi, false),
            font_bold: create_ui_font(dpi, true),
            brush_window: CreateSolidBrush(COLORREF(COLOR_WINDOW_BG)),
            brush_card: CreateSolidBrush(COLORREF(COLOR_CARD_BG)),
            brush_border: CreateSolidBrush(COLORREF(COLOR_BORDER)),
            brush_disabled: CreateSolidBrush(COLORREF(COLOR_DISABLED_BG)),
        }
    }

    unsafe fn update_dpi(&mut self, dpi: u32) {
        if self.dpi == dpi {
            return;
        }
        if !self.font.is_invalid() {
            let _ = DeleteObject(self.font);
        }
        if !self.font_bold.is_invalid() {
            let _ = DeleteObject(self.font_bold);
        }
        self.dpi = dpi;
        self.font = create_ui_font(dpi, false);
        self.font_bold = create_ui_font(dpi, true);
    }

    unsafe fn dispose(&mut self) {
        for object in [
            self.font.0,
            self.font_bold.0,
            self.brush_window.0,
            self.brush_card.0,
            self.brush_border.0,
            self.brush_disabled.0,
        ] {
            if !object.is_null() {
                let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(object));
            }
        }
    }
}

pub fn open(host: HWND) {
    unsafe {
        let existing = settings_hwnd();
        if !existing.is_invalid() && IsWindow(existing).as_bool() {
            let _ = SetForegroundWindow(existing);
            return;
        }

        let controls = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES,
        };
        let _ = InitCommonControlsEx(&controls);

        let dpi = GetDpiForWindow(host).max(96);
        let style = settings_window_style();
        let (width, height) = outer_size_for_client(dpi, style);
        let instance = native::module_instance();
        let icon = native::load_app_icon();

        let class_name = wide(CLASS);
        let definition = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(proc),
            hInstance: instance,
            hIcon: icon,
            hIconSm: icon,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassExW(&definition);

        let data = Box::into_raw(Box::new(SettingsWindowData::create(host, dpi)));
        let title = wide("CodexPlus 设置");
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(title.as_ptr()),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            width,
            height,
            host,
            None,
            instance,
            Some(data.cast()),
        );
        match hwnd {
            Ok(hwnd) => {
                SETTINGS_WINDOW.store(hwnd.0 as isize, Ordering::Release);
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetForegroundWindow(hwnd);
            }
            Err(_) => {
                let mut data = Box::from_raw(data);
                data.dispose();
            }
        }
    }
}

/// Gives the modeless native settings window standard dialog keyboard behavior.
pub fn pre_translate_message(message: &MSG) -> bool {
    unsafe {
        let hwnd = settings_hwnd();
        if hwnd.is_invalid() || !IsWindow(hwnd).as_bool() {
            return false;
        }
        IsDialogMessageW(hwnd, message).as_bool()
    }
}

fn settings_hwnd() -> HWND {
    HWND(SETTINGS_WINDOW.load(Ordering::Acquire) as *mut _)
}

fn settings_window_style() -> WINDOW_STYLE {
    WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN
}

unsafe fn outer_size_for_client(dpi: u32, style: WINDOW_STYLE) -> (i32, i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: scale(CLIENT_WIDTH, dpi),
        bottom: scale(CLIENT_HEIGHT, dpi),
    };
    if AdjustWindowRectExForDpi(&mut rect, style, false, WINDOW_EX_STYLE::default(), dpi).is_err() {
        return (scale(CLIENT_WIDTH, dpi), scale(CLIENT_HEIGHT, dpi));
    }
    (rect.right - rect.left, rect.bottom - rect.top)
}

fn scale(value: i32, dpi: u32) -> i32 {
    (value * dpi as i32 + 48) / 96
}

unsafe fn create_ui_font(dpi: u32, bold: bool) -> HFONT {
    let face = wide("Microsoft YaHei UI");
    let height_px = (9 * dpi as i32 + 36) / 72;
    let weight = if bold { 600 } else { FW_NORMAL.0 as i32 };
    CreateFontW(
        -height_px,
        0,
        0,
        0,
        weight,
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

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            let create = lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
            if !create.is_null() {
                let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
            }
            LRESULT(1)
        }
        WM_CREATE => {
            build_controls(hwnd);
            layout_controls(hwnd);
            let _ = SendMessageW(hwnd, DM_SETDEFID, WPARAM(ID_SAVE), LPARAM(0));
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
            let hdc = HDC(wparam.0 as *mut _);
            let edit = HWND(lparam.0 as *mut _);
            paint_edit(hwnd, hdc, edit)
        }
        WM_CTLCOLORSTATIC => {
            let hdc = HDC(wparam.0 as *mut _);
            let control = HWND(lparam.0 as *mut _);
            paint_static(hwnd, hdc, control)
        }
        WM_CTLCOLORBTN => get_data(hwnd)
            .map(|data| LRESULT(data.brush_card.0 as isize))
            .unwrap_or(LRESULT(0)),
        WM_DPICHANGED => {
            let new_dpi = ((wparam.0 >> 16) & 0xffff) as u32;
            if let Some(data) = get_data(hwnd) {
                data.update_dpi(new_dpi.max(96));
            }
            apply_fonts(hwnd);
            layout_controls(hwnd);
            let suggested = lparam.0 as *const RECT;
            if let Some(rect) = suggested.as_ref() {
                let _ = SetWindowPos(
                    hwnd,
                    HWND::default(),
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            let _ = InvalidateRect(hwnd, None, true);
            LRESULT(0)
        }
        WM_PAINT => {
            paint_chrome(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            SETTINGS_WINDOW
                .compare_exchange(hwnd.0 as isize, 0, Ordering::AcqRel, Ordering::Acquire)
                .ok();
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsWindowData;
            let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if !pointer.is_null() {
                let mut data = Box::from_raw(pointer);
                data.dispose();
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn get_data(hwnd: HWND) -> Option<&'static mut SettingsWindowData> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsWindowData).as_mut()
}

unsafe fn paint_edit(hwnd: HWND, hdc: HDC, edit: HWND) -> LRESULT {
    let Some(data) = get_data(hwnd) else {
        return LRESULT(0);
    };
    let disabled = (GetWindowLongPtrW(edit, GWL_STYLE) & WS_DISABLED.0 as isize) != 0;
    let brush = if disabled {
        data.brush_disabled
    } else {
        data.brush_card
    };
    let _ = SetBkMode(hdc, TRANSPARENT);
    let _ = SetTextColor(hdc, COLORREF(COLOR_TEXT));
    LRESULT(brush.0 as isize)
}

unsafe fn paint_static(hwnd: HWND, hdc: HDC, control: HWND) -> LRESULT {
    let Some(data) = get_data(hwnd) else {
        return LRESULT(0);
    };
    let id = GetDlgCtrlID(control);
    let page_text = matches!(id as usize, ID_PAGE_TITLE | ID_PAGE_SUBTITLE);
    let secondary = matches!(id as usize, ID_PAGE_SUBTITLE | ID_PROXY_HELP);
    let _ = SetBkMode(hdc, TRANSPARENT);
    let _ = SetTextColor(
        hdc,
        COLORREF(if secondary {
            COLOR_TEXT_SECONDARY
        } else {
            COLOR_TEXT
        }),
    );
    let brush = if page_text {
        data.brush_window
    } else {
        data.brush_card
    };
    LRESULT(brush.0 as isize)
}

unsafe fn paint_chrome(hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut paint);
    let Some(data) = get_data(hwnd) else {
        let _ = EndPaint(hwnd, &paint);
        return;
    };
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let _ = FillRect(hdc, &client, data.brush_window);

    let layout = SettingsLayout::new();
    draw_card(hdc, data, scaled_rect(layout.display_card, data.dpi));
    draw_card(hdc, data, scaled_rect(layout.proxy_card, data.dpi));
    let _ = EndPaint(hwnd, &paint);
}

unsafe fn draw_card(hdc: HDC, data: &SettingsWindowData, rect: RECT) {
    let _ = FillRect(hdc, &rect, data.brush_card);
    let _ = FrameRect(hdc, &rect, data.brush_border);
}

fn scaled_rect(rect: LogicalRect, dpi: u32) -> RECT {
    RECT {
        left: scale(rect.left, dpi),
        top: scale(rect.top, dpi),
        right: scale(rect.right(), dpi),
        bottom: scale(rect.bottom(), dpi),
    }
}

unsafe fn build_controls(hwnd: HWND) {
    label(hwnd, ID_PAGE_TITLE, "设置");
    label(
        hwnd,
        ID_PAGE_SUBTITLE,
        "管理标题栏用量显示、刷新频率和网络连接方式",
    );
    label(hwnd, ID_DISPLAY_TITLE, "显示与刷新");
    button(
        hwnd,
        "在 Codex 标题栏显示用量",
        BS_AUTOCHECKBOX as u32,
        ID_OVERLAY,
        WS_TABSTOP.0,
    );
    label(hwnd, ID_INTERVAL_LABEL, "刷新间隔（秒）");
    edit(hwnd, ID_INTERVAL, true);

    label(hwnd, ID_USAGE_LABEL, "额度显示");
    button(
        hwnd,
        "显示剩余额度",
        BS_AUTORADIOBUTTON as u32,
        ID_USAGE_REMAINING,
        WS_GROUP.0 | WS_TABSTOP.0,
    );
    button(
        hwnd,
        "显示已用额度",
        BS_AUTORADIOBUTTON as u32,
        ID_USAGE_USED,
        WS_TABSTOP.0,
    );

    label(hwnd, ID_RESET_LABEL, "重置时间显示");
    button(
        hwnd,
        "倒计时（如 5天14时）",
        BS_AUTORADIOBUTTON as u32,
        ID_RESET_COUNTDOWN,
        WS_GROUP.0 | WS_TABSTOP.0,
    );
    button(
        hwnd,
        "到期时间（如 7月30日 19:34）",
        BS_AUTORADIOBUTTON as u32,
        ID_RESET_ABSOLUTE,
        WS_TABSTOP.0,
    );

    label(hwnd, ID_PROXY_TITLE, "网络代理");
    button(
        hwnd,
        "使用系统环境代理",
        BS_AUTORADIOBUTTON as u32,
        ID_PROXY_ENV,
        WS_GROUP.0 | WS_TABSTOP.0,
    );
    button(
        hwnd,
        "强制直连（忽略所有代理）",
        BS_AUTORADIOBUTTON as u32,
        ID_PROXY_DIRECT,
        WS_TABSTOP.0,
    );
    button(
        hwnd,
        "使用自定义代理地址",
        BS_AUTORADIOBUTTON as u32,
        ID_PROXY_CUSTOM,
        WS_TABSTOP.0,
    );
    label(hwnd, ID_PROXY_ADDRESS_LABEL, "代理地址");
    edit(hwnd, ID_PROXY_ADDRESS, false);
    label(
        hwnd,
        ID_PROXY_HELP,
        "代理地址保存在本机配置文件中，打开设置时会再次显示。",
    );

    button(hwnd, "取消", BS_PUSHBUTTON as u32, ID_CANCEL, WS_TABSTOP.0);
    button(hwnd, "保存", BS_DEFPUSHBUTTON as u32, ID_SAVE, WS_TABSTOP.0);

    let current = settings::load();
    check(hwnd, ID_OVERLAY, current.overlay_enabled);
    set_text(
        child(hwnd, ID_INTERVAL),
        &current.refresh_interval_secs.to_string(),
    );
    let usage_id = match current.usage_display {
        UsageDisplay::Remaining => ID_USAGE_REMAINING,
        UsageDisplay::Used => ID_USAGE_USED,
    };
    check(hwnd, ID_USAGE_REMAINING, usage_id == ID_USAGE_REMAINING);
    check(hwnd, ID_USAGE_USED, usage_id == ID_USAGE_USED);

    let reset_id = match current.reset_time_display {
        ResetTimeDisplay::Countdown => ID_RESET_COUNTDOWN,
        ResetTimeDisplay::Absolute => ID_RESET_ABSOLUTE,
    };
    check(hwnd, ID_RESET_COUNTDOWN, reset_id == ID_RESET_COUNTDOWN);
    check(hwnd, ID_RESET_ABSOLUTE, reset_id == ID_RESET_ABSOLUTE);

    match current.proxy_mode {
        ProxyMode::Environment => check(hwnd, ID_PROXY_ENV, true),
        ProxyMode::Direct => check(hwnd, ID_PROXY_DIRECT, true),
        ProxyMode::Custom => check(hwnd, ID_PROXY_CUSTOM, true),
    }
    if let Some(url) = current.custom_proxy_url.as_deref() {
        if !url.is_empty() {
            set_text(child(hwnd, ID_PROXY_ADDRESS), url);
        }
    }

    apply_fonts(hwnd);
    apply_themes(hwnd);
    update_custom_enabled(hwnd);
}

unsafe fn layout_controls(hwnd: HWND) {
    let dpi = get_data(hwnd)
        .map(|data| data.dpi)
        .unwrap_or_else(|| GetDpiForWindow(hwnd).max(96));
    let layout = SettingsLayout::new();

    place(hwnd, ID_PAGE_TITLE, LogicalRect::new(20, 22, 520, 28), dpi);
    place(
        hwnd,
        ID_PAGE_SUBTITLE,
        LogicalRect::new(20, 54, 520, 20),
        dpi,
    );
    place(
        hwnd,
        ID_DISPLAY_TITLE,
        LogicalRect::new(40, 104, 480, 22),
        dpi,
    );
    place(hwnd, ID_OVERLAY, LogicalRect::new(40, 136, 480, 24), dpi);
    place(
        hwnd,
        ID_INTERVAL_LABEL,
        LogicalRect::new(40, 177, 260, 22),
        dpi,
    );
    place(hwnd, ID_INTERVAL, LogicalRect::new(380, 170, 140, 30), dpi);

    place(
        hwnd,
        ID_USAGE_LABEL,
        LogicalRect::new(40, 214, 480, 20),
        dpi,
    );
    place(
        hwnd,
        ID_USAGE_REMAINING,
        LogicalRect::new(40, 240, 220, 24),
        dpi,
    );
    place(
        hwnd,
        ID_USAGE_USED,
        LogicalRect::new(280, 240, 220, 24),
        dpi,
    );

    place(
        hwnd,
        ID_RESET_LABEL,
        LogicalRect::new(40, 278, 480, 20),
        dpi,
    );
    place(
        hwnd,
        ID_RESET_COUNTDOWN,
        LogicalRect::new(40, 304, 220, 24),
        dpi,
    );
    place(
        hwnd,
        ID_RESET_ABSOLUTE,
        LogicalRect::new(280, 304, 260, 24),
        dpi,
    );

    place(
        hwnd,
        ID_PROXY_TITLE,
        LogicalRect::new(40, 360, 480, 22),
        dpi,
    );
    place(hwnd, ID_PROXY_ENV, LogicalRect::new(40, 392, 480, 24), dpi);
    place(
        hwnd,
        ID_PROXY_DIRECT,
        LogicalRect::new(40, 424, 480, 24),
        dpi,
    );
    place(
        hwnd,
        ID_PROXY_CUSTOM,
        LogicalRect::new(40, 456, 480, 24),
        dpi,
    );
    place(
        hwnd,
        ID_PROXY_ADDRESS_LABEL,
        LogicalRect::new(40, 497, 480, 20),
        dpi,
    );
    place(
        hwnd,
        ID_PROXY_ADDRESS,
        LogicalRect::new(40, 522, 480, 32),
        dpi,
    );
    place(hwnd, ID_PROXY_HELP, LogicalRect::new(40, 560, 480, 18), dpi);

    place(hwnd, ID_CANCEL, layout.cancel_button, dpi);
    place(hwnd, ID_SAVE, layout.save_button, dpi);
}

unsafe fn place(hwnd: HWND, id: usize, rect: LogicalRect, dpi: u32) {
    let _ = SetWindowPos(
        child(hwnd, id),
        HWND::default(),
        scale(rect.left, dpi),
        scale(rect.top, dpi),
        scale(rect.width, dpi),
        scale(rect.height, dpi),
        SWP_NOZORDER | SWP_NOACTIVATE,
    );
}

unsafe fn apply_fonts(hwnd: HWND) {
    let Some(data) = get_data(hwnd) else {
        return;
    };
    for id in [ID_PAGE_TITLE, ID_DISPLAY_TITLE, ID_PROXY_TITLE] {
        set_font(child(hwnd, id), data.font_bold);
    }
    for id in [
        ID_PAGE_SUBTITLE,
        ID_OVERLAY,
        ID_INTERVAL_LABEL,
        ID_INTERVAL,
        ID_USAGE_LABEL,
        ID_USAGE_REMAINING,
        ID_USAGE_USED,
        ID_RESET_LABEL,
        ID_RESET_COUNTDOWN,
        ID_RESET_ABSOLUTE,
        ID_PROXY_ENV,
        ID_PROXY_DIRECT,
        ID_PROXY_CUSTOM,
        ID_PROXY_ADDRESS_LABEL,
        ID_PROXY_ADDRESS,
        ID_PROXY_HELP,
        ID_CANCEL,
        ID_SAVE,
    ] {
        set_font(child(hwnd, id), data.font);
    }
}

unsafe fn apply_themes(hwnd: HWND) {
    let explorer = wide("Explorer");
    for id in [
        ID_OVERLAY,
        ID_INTERVAL,
        ID_USAGE_REMAINING,
        ID_USAGE_USED,
        ID_RESET_COUNTDOWN,
        ID_RESET_ABSOLUTE,
        ID_PROXY_ENV,
        ID_PROXY_DIRECT,
        ID_PROXY_CUSTOM,
        ID_PROXY_ADDRESS,
        ID_CANCEL,
        ID_SAVE,
    ] {
        let _ = SetWindowTheme(child(hwnd, id), PCWSTR(explorer.as_ptr()), PCWSTR::null());
    }
}

unsafe fn save(hwnd: HWND) {
    let mode = if checked(hwnd, ID_PROXY_DIRECT) {
        ProxyMode::Direct
    } else if checked(hwnd, ID_PROXY_CUSTOM) {
        ProxyMode::Custom
    } else {
        ProxyMode::Environment
    };
    let usage_display = if checked(hwnd, ID_USAGE_USED) {
        UsageDisplay::Used
    } else {
        UsageDisplay::Remaining
    };
    let reset_time_display = if checked(hwnd, ID_RESET_ABSOLUTE) {
        ResetTimeDisplay::Absolute
    } else {
        ResetTimeDisplay::Countdown
    };
    let mut settings_value = settings::load();
    let address = text(hwnd, ID_PROXY_ADDRESS);
    let trimmed_address = address.trim().to_string();
    if mode == ProxyMode::Custom {
        if trimmed_address.is_empty() {
            show_invalid_proxy_message(hwnd);
            return;
        }
        if !settings::validate_custom_proxy_url(&trimmed_address) {
            show_invalid_proxy_message(hwnd);
            return;
        }
    }
    let interval = text(hwnd, ID_INTERVAL).parse::<u32>().unwrap_or(300);
    SettingsPatch {
        overlay_enabled: Some(checked(hwnd, ID_OVERLAY)),
        refresh_interval_secs: Some(interval),
        proxy_mode: Some(mode),
        custom_proxy_url: Some(trimmed_address),
        usage_display: Some(usage_display),
        reset_time_display: Some(reset_time_display),
    }
    .apply_to(&mut settings_value);
    if settings::save(&settings_value).is_err() {
        return;
    }
    if let Some(data) = get_data(hwnd) {
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

unsafe fn label(hwnd: HWND, id: usize, text_value: &str) {
    let class = wide("STATIC");
    let value = wide(text_value);
    let _ = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        PCWSTR(class.as_ptr()),
        PCWSTR(value.as_ptr()),
        WS_CHILD | WS_VISIBLE,
        0,
        0,
        0,
        0,
        hwnd,
        HMENU(id as *mut _),
        HINSTANCE::default(),
        None,
    );
}

unsafe fn button(hwnd: HWND, text_value: &str, style: u32, id: usize, extra_style: u32) {
    let class = wide("BUTTON");
    let value = wide(text_value);
    let _ = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        PCWSTR(class.as_ptr()),
        PCWSTR(value.as_ptr()),
        WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | style | extra_style),
        0,
        0,
        0,
        0,
        hwnd,
        HMENU(id as *mut _),
        HINSTANCE::default(),
        None,
    );
}

unsafe fn edit(hwnd: HWND, id: usize, digits: bool) {
    let class = wide("EDIT");
    let style = WS_CHILD.0
        | WS_VISIBLE.0
        | WS_TABSTOP.0
        | ES_AUTOHSCROLL as u32
        | if digits {
            windows::Win32::UI::WindowsAndMessaging::ES_NUMBER as u32
        } else {
            0
        };
    let _ = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        PCWSTR(class.as_ptr()),
        PCWSTR::null(),
        WINDOW_STYLE(style),
        0,
        0,
        0,
        0,
        hwnd,
        HMENU(id as *mut _),
        HINSTANCE::default(),
        None,
    );
}

unsafe fn set_font(hwnd: HWND, font: HFONT) {
    let _ = SendMessageW(
        hwnd,
        windows::Win32::UI::WindowsAndMessaging::WM_SETFONT,
        WPARAM(font.0 as usize),
        LPARAM(1),
    );
}

unsafe fn child(hwnd: HWND, id: usize) -> HWND {
    windows::Win32::UI::WindowsAndMessaging::GetDlgItem(hwnd, id as i32).unwrap_or_default()
}

unsafe fn check(hwnd: HWND, id: usize, value: bool) {
    let _ = SendMessageW(
        child(hwnd, id),
        BM_SETCHECK,
        WPARAM(if value {
            windows::Win32::UI::Controls::BST_CHECKED.0 as usize
        } else {
            0
        }),
        LPARAM(0),
    );
}

unsafe fn checked(hwnd: HWND, id: usize) -> bool {
    SendMessageW(child(hwnd, id), BM_GETCHECK, WPARAM(0), LPARAM(0)).0
        == windows::Win32::UI::Controls::BST_CHECKED.0 as isize
}

unsafe fn text(hwnd: HWND, id: usize) -> String {
    let mut buf = [0u16; 1024];
    let len = GetWindowTextW(child(hwnd, id), &mut buf);
    String::from_utf16_lossy(&buf[..len as usize])
}

unsafe fn set_text(hwnd: HWND, value: &str) {
    let value = wide(value);
    let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowTextW(hwnd, PCWSTR(value.as_ptr()));
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_layout_stays_inside_client_area() {
        let layout = SettingsLayout::new();
        for rect in [
            layout.display_card,
            layout.proxy_card,
            layout.cancel_button,
            layout.save_button,
        ] {
            assert!(rect.left >= 0);
            assert!(rect.top >= 0);
            assert!(rect.right() <= CLIENT_WIDTH);
            assert!(rect.bottom() <= CLIENT_HEIGHT);
        }
        assert!(layout.display_card.bottom() < layout.proxy_card.top);
        assert!(layout.proxy_card.bottom() < layout.save_button.top);
    }

    #[test]
    fn scaling_preserves_expected_dimensions() {
        for (dpi, expected_width, expected_height) in [
            (96, 560, 678),
            (120, 700, 848),
            (144, 840, 1017),
            (192, 1120, 1356),
        ] {
            assert_eq!(scale(CLIENT_WIDTH, dpi), expected_width);
            assert_eq!(scale(CLIENT_HEIGHT, dpi), expected_height);
            let card = scaled_rect(SettingsLayout::new().display_card, dpi);
            assert!(card.right <= expected_width);
            assert!(card.bottom <= expected_height);
        }
    }

    #[test]
    fn nine_point_font_is_twelve_pixels_at_standard_dpi() {
        assert_eq!((9 * 96 + 36) / 72, 12);
        assert_eq!((9 * 144 + 36) / 72, 18);
    }
}
