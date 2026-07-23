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
    DEFAULT_CHARSET, DIB_RGB_COLORS, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, PAINTSTRUCT,
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

/// Super-sampling factor used when rasterising the overlay text. Drawing
/// `DrawTextW` into a high-resolution off-screen DIB and down-sampling the
/// per-pixel coverage produces visibly smoother anti-aliased edges on a
/// transparent layered window, where the ClearType-friendly sub-pixel grid
/// is unavailable.
const TEXT_RENDER_SCALE: i32 = 3;

/// Average the per-pixel coverage of a `scale x scale` block into a single
/// 0..=255 alpha value. The downsampler rounds to nearest so that opaque
/// blocks stay fully covered instead of biasing toward transparent.
fn downsample_alpha_block(
    high_pixels: &[u8],
    render_width: usize,
    x: usize,
    y: usize,
    scale: usize,
) -> u8 {
    let mut sum: u32 = 0;
    let block_left = x * scale;
    let block_top = y * scale;
    for block_y in 0..scale {
        let row_offset = (block_top + block_y) * render_width * 4;
        for block_x in 0..scale {
            let pixel_offset = row_offset + (block_left + block_x) * 4;
            let b = high_pixels[pixel_offset];
            let g = high_pixels[pixel_offset + 1];
            let r = high_pixels[pixel_offset + 2];
            let coverage = (b.max(g)).max(r) as u32;
            sum += coverage;
        }
    }
    let samples = (scale * scale) as u32;
    ((sum + samples / 2) / samples).min(255) as u8
}

/// Convert per-pixel coverage into the pre-multiplied BGRA bytes expected by
/// `UpdateLayeredWindow(ULW_ALPHA)`. Each group of four bytes encodes
/// `B, G, R, A` where every colour channel is scaled by `alpha / 255` so a
/// fully transparent pixel collapses to `[0, 0, 0, 0]` and a fully opaque
/// pixel keeps the exact RGB payload.
fn build_premultiplied_pixels(coverages: &[u8], rgb: (u8, u8, u8)) -> Vec<u8> {
    let (red, green, blue) = rgb;
    let mut pixels = Vec::with_capacity(coverages.len() * 4);
    for &alpha in coverages {
        let a = alpha as u16;
        pixels.push(((blue as u16 * a) / 255) as u8);
        pixels.push(((green as u16 * a) / 255) as u8);
        pixels.push(((red as u16 * a) / 255) as u8);
        pixels.push(alpha);
    }
    pixels
}

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
    let tone_rgb = tone_text_rgb(row.tone);

    let scale = TEXT_RENDER_SCALE;
    let render_width = width.checked_mul(scale).ok_or(())?;
    let render_height = height.checked_mul(scale).ok_or(())?;
    let width_us = width as usize;
    let height_us = height as usize;
    let render_width_us = render_width as usize;
    let scale_us = scale as usize;

    let screen_dc = GetDC(HWND::default());
    if screen_dc.is_invalid() {
        return Err(());
    }
    let memory_dc = CreateCompatibleDC(screen_dc);
    if memory_dc.is_invalid() {
        let _ = ReleaseDC(HWND::default(), screen_dc);
        return Err(());
    }

    let render_byte_len = render_width_us * (render_height as usize) * 4;

    // High-resolution DIB used only for off-screen text rasterisation.
    let high_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: render_width,
            biHeight: -render_height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: render_byte_len as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut high_bits: *mut c_void = std::ptr::null_mut();
    let high_bitmap = match CreateDIBSection(
        memory_dc,
        &high_info,
        DIB_RGB_COLORS,
        &mut high_bits,
        HANDLE::default(),
        0,
    ) {
        Ok(handle) if !high_bits.is_null() => handle,
        _ => {
            let _ = DeleteDC(memory_dc);
            let _ = ReleaseDC(HWND::default(), screen_dc);
            return Err(());
        }
    };
    let previous_bitmap = SelectObject(memory_dc, high_bitmap);
    std::ptr::write_bytes(high_bits, 0, render_byte_len);

    let dpi = GetDpiForWindow(hwnd).max(96);
    let font_height = -((14_i32 * dpi as i32 * scale + 48) / 96);
    let face: Vec<u16> = "Segoe UI Variable Text"
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
    let padding = ((4_i32 * dpi as i32 * scale) + 48) / 96;
    let mut text_rect = RECT {
        left: 0,
        top: 0,
        right: render_width.saturating_sub(padding),
        bottom: render_height,
    };
    let _ = DrawTextW(
        memory_dc,
        &mut text,
        &mut text_rect,
        DT_RIGHT | DT_VCENTER | DT_SINGLELINE,
    );
    let _ = SelectObject(memory_dc, previous_font);
    let _ = DeleteObject(font);

    let high_pixels = std::slice::from_raw_parts(high_bits as *const u8, render_byte_len);
    let mut coverages = Vec::with_capacity(width_us * height_us);
    for y in 0..height_us {
        for x in 0..width_us {
            coverages.push(downsample_alpha_block(
                high_pixels,
                render_width_us,
                x,
                y,
                scale_us,
            ));
        }
    }

    // Release the high-resolution DIB before swapping the final DIB into the
    // same DC. UpdateLayeredWindow must see a bitmap whose stride matches
    // the window size exactly; writing compact target pixels into the
    // oversized buffer produced the vertically-compressed output bug.
    let _ = SelectObject(memory_dc, previous_bitmap);
    let _ = DeleteObject(high_bitmap);

    let final_byte_len = width_us * height_us * 4;
    let final_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: final_byte_len as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut final_bits: *mut c_void = std::ptr::null_mut();
    let final_bitmap = match CreateDIBSection(
        memory_dc,
        &final_info,
        DIB_RGB_COLORS,
        &mut final_bits,
        HANDLE::default(),
        0,
    ) {
        Ok(handle) if !final_bits.is_null() => handle,
        _ => {
            let _ = DeleteDC(memory_dc);
            let _ = ReleaseDC(HWND::default(), screen_dc);
            return Err(());
        }
    };
    let previous_final_bitmap = SelectObject(memory_dc, final_bitmap);

    let final_pixels = build_premultiplied_pixels(&coverages, tone_rgb);
    std::ptr::copy_nonoverlapping(final_pixels.as_ptr(), final_bits as *mut u8, final_byte_len);

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

    let _ = SelectObject(memory_dc, previous_final_bitmap);
    let _ = DeleteObject(final_bitmap);
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
        let margin = ((6_i32 * dpi as i32) + 48) / 96;
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

    fn build_block(value: u8, width: usize, height: usize) -> Vec<u8> {
        let mut pixels = vec![0u8; width * height * 4];
        for chunk in pixels.chunks_exact_mut(4) {
            chunk[0] = value;
            chunk[1] = value;
            chunk[2] = value;
        }
        pixels
    }

    #[test]
    fn downsample_alpha_block_zero_for_empty_block() {
        let pixels = build_block(0, 9, 9);
        assert_eq!(downsample_alpha_block(&pixels, 9, 0, 0, 3), 0);
        assert_eq!(downsample_alpha_block(&pixels, 9, 2, 2, 3), 0);
    }

    #[test]
    fn downsample_alpha_block_full_for_opaque_block() {
        let pixels = build_block(255, 9, 9);
        assert_eq!(downsample_alpha_block(&pixels, 9, 0, 0, 3), 255);
        assert_eq!(downsample_alpha_block(&pixels, 9, 2, 2, 3), 255);
    }

    #[test]
    fn downsample_alpha_block_averages_halftone() {
        // Fill a 9x3 high-res bitmap so the downsampler can read three
        // neighbouring 3x3 blocks. Each row is constant 51, so every
        // 3x3 block averages to 51.
        let render_width = 9;
        let mut pixels = vec![0u8; render_width * 3 * 4];
        for y in 0..3 {
            for x in 0..render_width {
                let offset = (y * render_width + x) * 4;
                pixels[offset] = 51;
                pixels[offset + 1] = 51;
                pixels[offset + 2] = 51;
            }
        }
        for block_x in 0..3 {
            assert_eq!(
                downsample_alpha_block(&pixels, render_width, block_x, 0, 3),
                51
            );
        }
    }

    #[test]
    fn downsample_alpha_block_rounds_half_to_nearest() {
        // 9x3 high-res bitmap where the first 3x3 block holds five pixels
        // of 255 and four of 0 → average = 141.666, rounded to 142.
        let render_width = 9;
        let mut pixels = vec![0u8; render_width * 3 * 4];
        let bright: [(usize, usize); 5] = [(0, 0), (0, 2), (1, 1), (2, 0), (2, 2)];
        for &(y, x) in &bright {
            let offset = (y * render_width + x) * 4;
            pixels[offset] = 255;
            pixels[offset + 1] = 255;
            pixels[offset + 2] = 255;
        }
        assert_eq!(downsample_alpha_block(&pixels, render_width, 0, 0, 3), 142);
        // The remaining blocks are all zero.
        assert_eq!(downsample_alpha_block(&pixels, render_width, 1, 0, 3), 0);
    }

    #[test]
    fn build_premultiplied_pixels_zero_alpha_is_transparent_black() {
        let pixels = build_premultiplied_pixels(&[0], (176, 40, 48));
        assert_eq!(pixels, vec![0, 0, 0, 0]);
    }

    #[test]
    fn build_premultiplied_pixels_full_alpha_preserves_rgb() {
        let pixels = build_premultiplied_pixels(&[255], (176, 40, 48));
        assert_eq!(pixels, vec![48, 40, 176, 255]);
    }

    #[test]
    fn build_premultiplied_pixels_half_alpha_premultiplies() {
        // (48 * 128) / 255 = 24; (40 * 128) / 255 = 20; (176 * 128) / 255 = 88.
        let pixels = build_premultiplied_pixels(&[128], (176, 40, 48));
        assert_eq!(pixels, vec![24, 20, 88, 128]);
    }

    #[test]
    fn build_premultiplied_pixels_preserves_pixel_order_and_length() {
        let pixels = build_premultiplied_pixels(&[0, 255, 128], (10, 20, 30));
        assert_eq!(pixels.len(), 12);
        assert_eq!(&pixels[0..4], &[0, 0, 0, 0]);
        assert_eq!(&pixels[4..8], &[30, 20, 10, 255]);
        // Third pixel: B = 30*128/255 = 15; G = 20*128/255 = 10; R = 10*128/255 = 5.
        assert_eq!(&pixels[8..12], &[15, 10, 5, 128]);
    }
}
