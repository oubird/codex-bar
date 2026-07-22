//! Actual no-activate, mouse-transparent title-bar overlay window.

#![cfg(windows)]

use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    InvalidateRect, SelectObject, SetBkMode, SetTextColor, DEFAULT_CHARSET, DT_CENTER,
    DT_SINGLELINE, DT_VCENTER, FW_NORMAL, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetWindowLongPtrW,
    GetWindowRect, RegisterClassExW, SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HWND_TOPMOST, LWA_ALPHA, MA_NOACTIVATE,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND,
    WM_MOUSEACTIVATE, WM_NCHITTEST, WM_PAINT, WNDCLASSEXW, WS_POPUP,
};

use super::native;
use super::render::{build_text, Tone, OVERLAY_LOGICAL_HEIGHT, OVERLAY_LOGICAL_WIDTH};
use crate::formatting::format_tray_tooltip;
use crate::model::ViewState;

const OVERLAY_CLASS: &str = "CodexPlusOverlayWindow";
const HTTRANSPARENT: isize = -1;

struct OverlayData {
    state: ViewState,
    visible: bool,
}

pub struct OverlayHandle {
    hwnd: HWND,
}

impl OverlayHandle {
    pub fn create() -> Result<Self, String> {
        unsafe {
            let class = native::wide_str(OVERLAY_CLASS);
            let window_class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(overlay_proc),
                hInstance: Default::default(),
                hbrBackground: Default::default(),
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            };
            let _ = RegisterClassExW(&window_class);
            let data = Box::new(OverlayData {
                state: ViewState::Loading { cached: None },
                visible: false,
            });
            let data_ptr = Box::into_raw(data);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(native::overlay_extended_styles_bits()),
                PCWSTR(class.as_ptr()),
                PCWSTR::null(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                HWND::default(),
                None,
                HINSTANCE::default(),
                Some(data_ptr.cast()),
            )
            .map_err(|_| "Could not create usage overlay.".to_string())?;
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 225, LWA_ALPHA);
            Ok(Self { hwnd })
        }
    }

    pub fn set_state(&self, state: ViewState) {
        unsafe {
            if let Some(data) = data_for(self.hwnd) {
                data.state = state;
                let _ = InvalidateRect(self.hwnd, None, false);
            }
        }
    }

    pub fn tooltip(&self, now_unix: i64) -> String {
        unsafe {
            data_for(self.hwnd)
                .map(|d| format_tray_tooltip(&d.state, now_unix))
                .unwrap_or_default()
        }
    }

    pub fn sync_to_target(&self, target: Option<HWND>, enabled: bool) {
        let Some(target) = target else {
            self.hide();
            return;
        };
        if !enabled || !is_target_displayable(target) {
            self.hide();
            return;
        }
        unsafe {
            let Some(data) = data_for(self.hwnd) else {
                return;
            };
            if !data.state.is_displayable() {
                self.hide();
                return;
            }
        }
        let Some(rect) = target_overlay_rect(target) else {
            self.hide();
            return;
        };
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            if let Some(data) = data_for(self.hwnd) {
                data.visible = true;
            }
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            if let Some(data) = data_for(self.hwnd) {
                data.visible = false;
            }
        }
    }

    pub fn is_visible(&self) -> bool {
        unsafe { data_for(self.hwnd).is_some_and(|data| data.visible) }
    }

    pub fn destroy(&self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

unsafe fn data_for(hwnd: HWND) -> Option<&'static mut OverlayData> {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayData;
    pointer.as_mut()
}

unsafe extern "system" fn overlay_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        windows::Win32::UI::WindowsAndMessaging::WM_NCCREATE => {
            let create = lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
            if !create.is_null() {
                let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
            }
            LRESULT(1)
        }
        WM_PAINT => {
            paint_overlay(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_NCHITTEST => LRESULT(HTTRANSPARENT),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_CLOSE => {
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_DESTROY => {
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayData;
            if !pointer.is_null() {
                let _ = Box::from_raw(pointer);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe fn paint_overlay(hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let dc = BeginPaint(hwnd, &mut paint);
    let mut rect = RECT::default();
    let _ = GetClientRect(hwnd, &mut rect);
    let Some(data) = data_for(hwnd) else {
        let _ = EndPaint(hwnd, &paint);
        return;
    };
    let row = build_text(&data.state, crate::model::UsageSnapshot::now());
    let background = match row.tone {
        Tone::Ok => COLORREF(0x282828),
        Tone::Warn => COLORREF(0x214675),
        Tone::Critical => COLORREF(0x3030A0),
        Tone::Stale => COLORREF(0x555555),
    };
    let foreground = COLORREF(0xF0F0F0);
    let brush = CreateSolidBrush(background);
    let _ = FillRect(dc, &rect, brush);
    let _ = DeleteObject(brush);
    let dpi = GetDpiForWindow(hwnd).max(96);
    let font_height = -((12_i32 * dpi as i32 + 48) / 96);
    let face: Vec<u16> = "Segoe UI".encode_utf16().chain(Some(0)).collect();
    let font = CreateFontW(
        font_height,
        0,
        0,
        0,
        FW_NORMAL.0 as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        0,
        0,
        0,
        0,
        PCWSTR(face.as_ptr()),
    );
    let previous = SelectObject(dc, font);
    let _ = SetBkMode(dc, TRANSPARENT);
    let _ = SetTextColor(dc, foreground);
    let mut text: Vec<u16> = row.text.encode_utf16().collect();
    let _ = DrawTextW(
        dc,
        &mut text,
        &mut rect,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
    let _ = SelectObject(dc, previous);
    let _ = DeleteObject(font);
    let _ = EndPaint(hwnd, &paint);
}

pub fn target_overlay_rect(target: HWND) -> Option<RECT> {
    unsafe {
        let mut frame = RECT::default();
        GetWindowRect(target, &mut frame).ok()?;
        let dpi = GetDpiForWindow(target).max(96);
        let width = (OVERLAY_LOGICAL_WIDTH * dpi as i32 + 48) / 96;
        let height = (OVERLAY_LOGICAL_HEIGHT * dpi as i32 + 48) / 96;
        let caption_button = ((46_i32 * dpi as i32) + 48) / 96;
        let margin = ((8_i32 * dpi as i32) + 48) / 96;
        let available = frame.right - frame.left - caption_button * 3 - margin;
        if available < width {
            return None;
        }
        let left = frame.right - caption_button * 3 - margin - width;
        Some(RECT {
            left,
            top: frame.top + margin / 2,
            right: left + width,
            bottom: frame.top + margin / 2 + height,
        })
    }
}

pub fn is_target_displayable(target: HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::IsIconic;
    if target.is_invalid() || !native::is_window_visible(target) {
        return false;
    }
    unsafe {
        !IsIconic(target).as_bool()
            && native::window_rect(target)
                .is_some_and(|rect| rect.right > rect.left && rect.bottom > rect.top)
    }
}
