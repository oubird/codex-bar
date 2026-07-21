//! Overlay module — pure logic + an OS-private Win32 host.
//!
//! The pure logic (placement math, viewstate → text/tone mapping) lives
//! here and is unit-tested. The actual `HWND` lifecycle is delegated to
//! the Win32 host via the [`crate::platform`] trait, so other platforms
//! (or future `wgpu`-based renders) can swap in different implementations
//! without touching the logic.
//!
//! In the MVP scaffolding the Win32 host is a thin stub; the full
//! implementation lands during the Windows-only end-to-end pass.

#![cfg(windows)]

use std::sync::Arc;
use std::sync::Mutex;

use windows::Win32::Foundation::{HWND, RECT};

use super::native;
use super::render::{Tone, OVERLAY_LOGICAL_HEIGHT, OVERLAY_LOGICAL_WIDTH};
use crate::formatting::format_tray_tooltip;
use crate::model::ViewState;

/// Platform-specific overlay host. The MVP returns a stub; the production
/// implementation will own the actual HWND and Win32 paint loop.
pub trait OverlayHost: Send + Sync {
    fn create(&self, state: ViewState) -> Result<OverlayHandle, String>;
}

#[derive(Clone)]
pub struct OverlayHandle {
    inner: Arc<Mutex<OverlayHandleInner>>,
}

struct OverlayHandleInner {
    state: ViewState,
}

impl OverlayHandle {
    pub fn new(initial: ViewState) -> Self {
        Self {
            inner: Arc::new(Mutex::new(OverlayHandleInner { state: initial })),
        }
    }

    pub fn set_state(&self, state: ViewState) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.state = state;
        }
    }

    pub fn state(&self) -> ViewState {
        self.inner.lock().map(|g| g.state.clone()).unwrap_or(ViewState::Loading { cached: None })
    }

    pub fn tooltip(&self, now_unix: i64) -> String {
        format_tray_tooltip(&self.state(), now_unix)
    }

    pub fn reposition_to(&self, _target: HWND, _w: i32, _h: i32) -> bool {
        // The MVP stub can't move a non-existent HWND. Production code
        // will call `SetWindowPos` against the platform-allocated HWND.
        false
    }

    pub fn show(&self) -> bool {
        true
    }

    pub fn hide(&self) -> bool {
        true
    }

    pub fn is_visible(&self) -> bool {
        false
    }

    pub fn destroy(self) {}
}

/// MVP stub host. The real Win32 host will live in a sibling module.
pub struct StubOverlayHost;

impl OverlayHost for StubOverlayHost {
    fn create(&self, state: ViewState) -> Result<OverlayHandle, String> {
        Ok(OverlayHandle::new(state))
    }
}

/// Compute the title-bar-safe logical rectangle inside the target
/// window's frame. Returns `None` if there isn't enough room.
pub fn target_overlay_rect(target: HWND, overlay_width: i32, overlay_height: i32) -> Option<RECT> {
    use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
    unsafe {
        if target.0.is_null() {
            return None;
        }
        let mut frame = RECT::default();
        if windows::Win32::UI::WindowsAndMessaging::GetWindowRect(target, &mut frame).is_err() {
            return None;
        }
        let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(target);
        let scale = if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 };
        let border = (GetSystemMetricsForDpi(unsafe { windows::Win32::UI::WindowsAndMessaging::SM_CXPADDEDBORDER }, dpi)
            + GetSystemMetricsForDpi(windows::Win32::UI::WindowsAndMessaging::SM_CXFRAME, dpi)) as f64;
        let cx_button = GetSystemMetricsForDpi(windows::Win32::UI::WindowsAndMessaging::SM_CXSIZE, dpi) as f64;
        let border_px = border / scale;
        let button_px = cx_button / scale;
        let reserved = border_px + 3.0 * button_px + 8.0;
        let overlay_w = overlay_width as f64;
        let width_px = (frame.right - frame.left) as f64 / scale;
        if width_px - reserved - overlay_w < 0.0 {
            return None;
        }
        let left = (frame.right as f64 / scale - reserved - overlay_w) as i32;
        let top = (frame.top as f64 / scale + (border_px / 2.0)) as i32;
        Some(RECT {
            left,
            top,
            right: left + overlay_width,
            bottom: top + overlay_height,
        })
    }
}

#[allow(dead_code)]
pub fn is_target_displayable(target: HWND) -> bool {
    if target.0.is_null() {
        return false;
    }
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsZoomed};
    unsafe {
        if !native::is_window_visible(target) {
            return false;
        }
        if IsIconic(target).as_bool() {
            return false;
        }
        if IsZoomed(target).as_bool() {
            return false;
        }
    }
    native::window_rect(target)
        .map_or(false, |r| r.right - r.left > 0 && r.bottom - r.top > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RateWindow, UsageSnapshot, ViewState, WindowKind, FIVE_HOUR_SECONDS, WEEKLY_SECONDS};

    fn snap(five: f64, week: f64) -> UsageSnapshot {
        UsageSnapshot {
            five_hour: RateWindow {
                kind: WindowKind::FiveHour,
                used_percent: five,
                window_seconds: FIVE_HOUR_SECONDS,
                resets_at_unix: UsageSnapshot::now() + 7200,
            },
            weekly: RateWindow {
                kind: WindowKind::Weekly,
                used_percent: week,
                window_seconds: WEEKLY_SECONDS,
                resets_at_unix: UsageSnapshot::now() + 3 * 24 * 3600,
            },
            fetched_at_unix: UsageSnapshot::now(),
        }
    }

    #[test]
    fn overlay_dimensions_are_positive() {
        assert!(OVERLAY_LOGICAL_WIDTH >= 200);
        assert!(OVERLAY_LOGICAL_HEIGHT >= 24);
    }

    #[test]
    fn overlay_handle_reports_state() {
        let handle = OverlayHandle::new(ViewState::Loading { cached: None });
        match handle.state() {
            ViewState::Loading { .. } => {}
            _ => panic!("expected Loading"),
        }
        let snap = snap(42.0, 67.0);
        handle.set_state(ViewState::Ready { snapshot: snap });
        match handle.state() {
            ViewState::Ready { .. } => {}
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn overlay_handle_tooltip_contains_dash() {
        let handle = OverlayHandle::new(ViewState::Ready {
            snapshot: snap(42.0, 67.0),
        });
        let tip = handle.tooltip(UsageSnapshot::now());
        assert!(tip.contains('5') || tip.contains('h'));
    }

    #[test]
    fn tone_progresses_with_used() {
        assert_eq!(Tone::from_state(&ViewState::Ready { snapshot: snap(40.0, 40.0) }), Tone::Ok);
        assert_eq!(Tone::from_state(&ViewState::Ready { snapshot: snap(80.0, 80.0) }), Tone::Warn);
        assert_eq!(Tone::from_state(&ViewState::Ready { snapshot: snap(95.0, 95.0) }), Tone::Critical);
    }
}
