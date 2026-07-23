//! Actual no-activate, mouse-transparent title-bar overlay window.

#![cfg(windows)]

use std::ffi::c_void;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    COLORREF, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject,
    DrawTextW, EndPaint, GetDC, ReleaseDC, SelectObject, SetBkMode, SetTextColor, AC_SRC_ALPHA,
    AC_SRC_OVER, ANTIALIASED_QUALITY, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
    DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CENTER, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, PAINTSTRUCT,
    TRANSPARENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, GetWindowRect,
    RegisterClassExW, SetWindowLongPtrW, SetWindowPos, ShowWindow, UpdateLayeredWindow, CS_HREDRAW,
    CS_VREDRAW, GWLP_HWNDPARENT, GWLP_USERDATA, HWND_TOP, MA_NOACTIVATE, SWP_NOACTIVATE,
    SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE, ULW_ALPHA, WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY,
    WM_ERASEBKGND, WM_MOUSEACTIVATE, WM_NCHITTEST, WM_PAINT, WNDCLASSEXW, WS_POPUP,
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
    owner: HWND,
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
                owner: HWND::default(),
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
            Ok(Self { hwnd })
        }
    }

    pub fn set_state(&self, state: ViewState) {
        unsafe {
            if let Some(data) = data_for(self.hwnd) {
                data.state = state;
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

    pub fn sync_to_target(&self, target: Option<HWND>, enabled: bool, target_is_foreground: bool) {
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
            if data.owner != target {
                let _ = SetWindowLongPtrW(self.hwnd, GWLP_HWNDPARENT, target.0 as isize);
                data.owner = target;
            }
        }
        let Some(rect) = target_overlay_rect(target) else {
            self.hide();
            return;
        };

        unsafe {
            if render_transparent_text(self.hwnd, rect).is_err() {
                self.hide();
                return;
            }

            let insert_after = if target_is_foreground {
                HWND_TOP
            } else {
                HWND::default()
            };
            let mut flags = SWP_NOACTIVATE | SWP_SHOWWINDOW;
            if !target_is_foreground {
                flags |= SWP_NOZORDER;
            }
            let _ = SetWindowPos(
                self.hwnd,
                insert_after,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                flags,
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
            let mut paint = PAINTSTRUCT::default();
            let _ = BeginPaint(hwnd, &mut paint);
            let _ = EndPaint(hwnd, &paint);
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

unsafe fn render_transparent_text(hwnd: HWND, destination: RECT) -> Result<(), ()> {
    let width = destination.right - destination.left;
    let height = destination.bottom - destination.top;
    if width <= 0 || height <= 0 {
        return Err(());
    }
    let Some(data) = data_for(hwnd) else {
        return Err(());
    };
    let row = build_text(&data.state, crate::model::UsageSnapshot::now());
    let (red, green, blue) = tone_text_rgb(row.tone);

    let screen_dc = GetDC(HWND::default());
    if screen_dc.is_invalid() {
        return Err(());
    }
    let memory_dc = CreateCompatibleDC(screen_dc);
    if memory_dc.is_invalid() {
        let _ = ReleaseDC(HWND::default(), screen_dc);
        return Err(());
    }

    let bitmap_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: (width * height * 4) as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    let bitmap = match CreateDIBSection(
        memory_dc,
        &bitmap_info,
        DIB_RGB_COLORS,
        &mut bits,
        HANDLE::default(),
        0,
    ) {
        Ok(bitmap) if !bits.is_null() => bitmap,
        _ => {
            let _ = DeleteDC(memory_dc);
            let _ = ReleaseDC(HWND::default(), screen_dc);
            return Err(());
        }
    };
    let previous_bitmap = SelectObject(memory_dc, bitmap);

    let byte_len = (width as usize) * (height as usize) * 4;
    std::ptr::write_bytes(bits, 0, byte_len);

    let dpi = GetDpiForWindow(hwnd).max(96);
    let font_height = -((12_i32 * dpi as i32 + 48) / 96);
    let face: Vec<u16> = "Microsoft YaHei UI"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
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
        ANTIALIASED_QUALITY.0 as u32,
        0,
        PCWSTR(face.as_ptr()),
    );
    let previous_font = SelectObject(memory_dc, font);
    let _ = SetBkMode(memory_dc, TRANSPARENT);
    let _ = SetTextColor(memory_dc, COLORREF(0x00FF_FFFF));
    let mut text: Vec<u16> = row.text.encode_utf16().collect();
    let mut text_rect = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    let _ = DrawTextW(
        memory_dc,
        &mut text,
        &mut text_rect,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
    let _ = SelectObject(memory_dc, previous_font);
    let _ = DeleteObject(font);

    let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, byte_len);
    for pixel in pixels.chunks_exact_mut(4) {
        let coverage = pixel[0].max(pixel[1]).max(pixel[2]);
        pixel[0] = ((blue as u16 * coverage as u16) / 255) as u8;
        pixel[1] = ((green as u16 * coverage as u16) / 255) as u8;
        pixel[2] = ((red as u16 * coverage as u16) / 255) as u8;
        pixel[3] = coverage;
    }

    let destination_point = POINT {
        x: destination.left,
        y: destination.top,
    };
    let source_point = POINT { x: 0, y: 0 };
    let size = SIZE {
        cx: width,
        cy: height,
    };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let update_result = UpdateLayeredWindow(
        hwnd,
        screen_dc,
        Some(&destination_point),
        Some(&size),
        memory_dc,
        Some(&source_point),
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
    );

    let _ = SelectObject(memory_dc, previous_bitmap);
    let _ = DeleteObject(bitmap);
    let _ = DeleteDC(memory_dc);
    let _ = ReleaseDC(HWND::default(), screen_dc);
    update_result.map_err(|_| ())
}

fn tone_text_rgb(tone: Tone) -> (u8, u8, u8) {
    match tone {
        Tone::Ok => (82, 88, 96),
        Tone::Warn => (154, 91, 0),
        Tone::Critical => (176, 40, 48),
        Tone::Stale => (112, 118, 126),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_style_is_not_globally_topmost() {
        assert_eq!(native::overlay_extended_styles_bits() & 0x00000008, 0);
    }

    #[test]
    fn tone_colors_are_distinct_and_native_like() {
        assert_eq!(tone_text_rgb(Tone::Ok), (82, 88, 96));
        assert_ne!(tone_text_rgb(Tone::Warn), tone_text_rgb(Tone::Critical));
        assert_ne!(tone_text_rgb(Tone::Ok), tone_text_rgb(Tone::Stale));
    }
}
