//! Clickable, no-activate overlay button placed inside Codex's composer row.

#![cfg(windows)]

use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, CreateSolidBrush, DeleteObject, Ellipse, EndPaint, FillRect,
    GetStockObject, InvalidateRect, LineTo, MoveToEx, SelectObject, NULL_BRUSH, PAINTSTRUCT,
    PS_SOLID,
};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetWindowLongPtrW,
    GetWindowRect, RegisterClassExW, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow,
    GWLP_HWNDPARENT, GWLP_USERDATA, HWND_TOP, MA_NOACTIVATE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SW_HIDE, WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_LBUTTONUP, WM_MOUSEACTIVATE,
    WM_MOUSEMOVE, WM_NCCREATE, WM_PAINT, WNDCLASSEXW, WS_POPUP,
};

use super::native;
use crate::settings::Language;

pub const WM_APP_REVIEW_CLICKED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 12;
pub const WM_APP_PLAN_CLICKED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 13;
pub const WM_APP_PROMPT_CLICKED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 14;

const REVIEW_BUTTON_CLASS: &str = "CodexPlusReviewButton";

struct ReviewButtonData {
    host: HWND,
    owner: HWND,
    enabled: bool,
    hovered: bool,
    language: Language,
    action: QuickAction,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum QuickAction {
    Prompt,
    Plan,
    Review,
}

pub struct ActionButtonHandle {
    hwnd: HWND,
}

impl ActionButtonHandle {
    pub fn create(host: HWND, action: QuickAction) -> Result<Self, String> {
        unsafe {
            let class = native::wide_str(REVIEW_BUTTON_CLASS);
            let window_class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(button_proc),
                hInstance: native::module_instance(),
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            };
            let _ = RegisterClassExW(&window_class);
            let data = Box::new(ReviewButtonData {
                host,
                owner: HWND::default(),
                enabled: false,
                hovered: false,
                language: Language::SimplifiedChinese,
                action,
            });
            let data_ptr = Box::into_raw(data);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0x00000080 | 0x08000000), // TOOLWINDOW | NOACTIVATE
                PCWSTR(class.as_ptr()),
                PCWSTR(
                    native::wide_str(accessible_title(action, Language::SimplifiedChinese))
                        .as_ptr(),
                ),
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
            .map_err(|_| "Could not create review button overlay.".to_string())?;
            Ok(Self { hwnd })
        }
    }

    pub fn sync(&self, target: HWND, rect: RECT, enabled: bool, language: Language) {
        unsafe {
            let Some(data) = data_for(self.hwnd) else {
                return;
            };
            if data.owner != target {
                let _ = SetWindowLongPtrW(self.hwnd, GWLP_HWNDPARENT, target.0 as isize);
                data.owner = target;
            }
            if data.enabled != enabled || data.language != language {
                data.enabled = enabled;
                data.language = language;
                let title = accessible_title(data.action, language);
                let title = native::wide_str(title);
                let _ = SetWindowTextW(self.hwnd, PCWSTR(title.as_ptr()));
                let _ = InvalidateRect(self.hwnd, None, true);
            }
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOP,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }

    pub fn hide(&self) {
        unsafe {
            if let Some(data) = data_for(self.hwnd) {
                data.hovered = false;
            }
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub fn destroy(&self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn screen_rect(&self) -> Option<RECT> {
        unsafe {
            let mut rect = RECT::default();
            GetWindowRect(self.hwnd, &mut rect).ok().map(|_| rect)
        }
    }
}

unsafe fn data_for(hwnd: HWND) -> Option<&'static mut ReviewButtonData> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ReviewButtonData).as_mut()
}

unsafe extern "system" fn button_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => {
            let create = lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
            if !create.is_null() {
                let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
            }
            LRESULT(1)
        }
        WM_PAINT => {
            paint_button(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_MOUSEMOVE => {
            if let Some(data) = data_for(hwnd) {
                if !data.hovered {
                    data.hovered = true;
                    let mut tracking = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tracking);
                    let _ = InvalidateRect(hwnd, None, true);
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            if let Some(data) = data_for(hwnd) {
                data.hovered = false;
                let _ = InvalidateRect(hwnd, None, true);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if let Some(data) = data_for(hwnd) {
                if data.enabled {
                    let message = match data.action {
                        QuickAction::Prompt => WM_APP_PROMPT_CLICKED,
                        QuickAction::Plan => WM_APP_PLAN_CLICKED,
                        QuickAction::Review => WM_APP_REVIEW_CLICKED,
                    };
                    let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                        data.host,
                        message,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_DESTROY => {
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ReviewButtonData;
            if !pointer.is_null() {
                let _ = Box::from_raw(pointer);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe fn paint_button(hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let dc = BeginPaint(hwnd, &mut paint);
    let mut rect = RECT::default();
    let _ = GetClientRect(hwnd, &mut rect);
    let Some(data) = data_for(hwnd) else {
        let _ = EndPaint(hwnd, &paint);
        return;
    };

    let (background, icon) = if data.enabled {
        let background = if data.hovered {
            COLORREF(0x00F1_F1F1)
        } else {
            COLORREF(0x00FF_FFFF)
        };
        (background, COLORREF(0x0064_6464))
    } else {
        (COLORREF(0x00FF_FFFF), COLORREF(0x00B8_B8B8))
    };
    let background_brush = CreateSolidBrush(background);
    let _ = FillRect(dc, &rect, background_brush);
    match data.action {
        QuickAction::Prompt => draw_prompt_icon(dc, rect, icon),
        QuickAction::Plan => draw_plan_icon(dc, rect, icon),
        QuickAction::Review => draw_review_icon(dc, rect, icon),
    }
    let _ = DeleteObject(background_brush);
    let _ = EndPaint(hwnd, &paint);
}

fn accessible_title(action: QuickAction, language: Language) -> &'static str {
    match (action, language) {
        (QuickAction::Prompt, Language::SimplifiedChinese) => "快捷指令",
        (QuickAction::Prompt, Language::English) => "Quick prompts",
        (QuickAction::Plan, Language::SimplifiedChinese) => "制定计划",
        (QuickAction::Plan, Language::English) => "Create plan",
        (QuickAction::Review, Language::SimplifiedChinese) => "审查所有未提交的改动",
        (QuickAction::Review, Language::English) => "Review all uncommitted changes",
    }
}

unsafe fn draw_prompt_icon(dc: windows::Win32::Graphics::Gdi::HDC, rect: RECT, color: COLORREF) {
    let center_x = (rect.left + rect.right) / 2;
    let center_y = (rect.top + rect.bottom) / 2;
    let pen = CreatePen(PS_SOLID, 1, color);
    let previous_pen = SelectObject(dc, pen);
    let previous_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));

    // Command-prompt chevron plus cursor, drawn without a font dependency.
    let _ = MoveToEx(dc, center_x - 7, center_y - 5, None);
    let _ = LineTo(dc, center_x - 2, center_y);
    let _ = LineTo(dc, center_x - 7, center_y + 5);
    let _ = MoveToEx(dc, center_x, center_y + 5, None);
    let _ = LineTo(dc, center_x + 7, center_y + 5);

    let _ = SelectObject(dc, previous_brush);
    let _ = SelectObject(dc, previous_pen);
    let _ = DeleteObject(pen);
}

unsafe fn draw_plan_icon(dc: windows::Win32::Graphics::Gdi::HDC, rect: RECT, color: COLORREF) {
    let center_x = (rect.left + rect.right) / 2;
    let center_y = (rect.top + rect.bottom) / 2;
    let pen = CreatePen(PS_SOLID, 1, color);
    let previous_pen = SelectObject(dc, pen);
    let previous_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));

    // A compact checklist communicates "plan" without relying on a font glyph.
    let _ = MoveToEx(dc, center_x - 7, center_y - 7, None);
    let _ = LineTo(dc, center_x + 7, center_y - 7);
    let _ = LineTo(dc, center_x + 7, center_y + 7);
    let _ = LineTo(dc, center_x - 7, center_y + 7);
    let _ = LineTo(dc, center_x - 7, center_y - 7);
    for row in [-3, 2] {
        let _ = MoveToEx(dc, center_x - 4, center_y + row, None);
        let _ = LineTo(dc, center_x - 2, center_y + row + 2);
        let _ = LineTo(dc, center_x + 1, center_y + row - 2);
        let _ = MoveToEx(dc, center_x + 2, center_y + row, None);
        let _ = LineTo(dc, center_x + 5, center_y + row);
    }

    let _ = SelectObject(dc, previous_brush);
    let _ = SelectObject(dc, previous_pen);
    let _ = DeleteObject(pen);
}

unsafe fn draw_review_icon(dc: windows::Win32::Graphics::Gdi::HDC, rect: RECT, color: COLORREF) {
    let center_x = (rect.left + rect.right) / 2;
    let center_y = (rect.top + rect.bottom) / 2;
    let pen = CreatePen(PS_SOLID, 1, color);
    let previous_pen = SelectObject(dc, pen);
    let previous_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));

    // Document outline with a folded top-right corner.
    let _ = MoveToEx(dc, center_x - 7, center_y - 7, None);
    let _ = LineTo(dc, center_x - 1, center_y - 7);
    let _ = LineTo(dc, center_x + 2, center_y - 4);
    let _ = LineTo(dc, center_x + 2, center_y);
    let _ = MoveToEx(dc, center_x - 7, center_y - 7, None);
    let _ = LineTo(dc, center_x - 7, center_y + 6);
    let _ = LineTo(dc, center_x - 1, center_y + 6);
    let _ = MoveToEx(dc, center_x - 1, center_y - 7, None);
    let _ = LineTo(dc, center_x - 1, center_y - 4);
    let _ = LineTo(dc, center_x + 2, center_y - 4);

    // Magnifier communicates review/search without relying on a font glyph.
    let _ = Ellipse(dc, center_x - 1, center_y - 1, center_x + 6, center_y + 6);
    let _ = MoveToEx(dc, center_x + 5, center_y + 5, None);
    let _ = LineTo(dc, center_x + 8, center_y + 8);

    let _ = SelectObject(dc, previous_brush);
    let _ = SelectObject(dc, previous_pen);
    let _ = DeleteObject(pen);
}
