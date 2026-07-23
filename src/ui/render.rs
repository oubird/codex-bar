//! Pure rendering helpers — the actual GDI paint happens inside
//! [`crate::ui::overlay`]. Keeping the paint decomposition in one place
//! makes it easy to test format mappings and colours without spinning up
//! the Windows runtime.

#![cfg(windows)]

use crate::formatting::{format_countdown_compact, format_remaining_percent, format_window_label};
use crate::model::ViewState;

/// Logical (DIP) size of the overlay — sized for `5小时 42% 1时18分   周限额 67% 3天4时`.
pub const OVERLAY_LOGICAL_WIDTH: i32 = 340;
pub const OVERLAY_LOGICAL_HEIGHT: i32 = 30;

#[derive(Debug, Clone)]
pub struct RenderedRow {
    pub text: String,
    pub tone: Tone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
    Critical,
    Stale,
}

impl Tone {
    pub fn from_state(state: &ViewState) -> Self {
        if state.stale_marker() {
            return Tone::Stale;
        }
        let Some(snap) = state.cached() else {
            return Tone::Stale;
        };
        let worst = snap
            .windows()
            .map(|window| window.used_percent)
            .fold(0.0_f64, f64::max);
        if worst >= 90.0 {
            Tone::Critical
        } else if worst >= 70.0 {
            Tone::Warn
        } else {
            Tone::Ok
        }
    }
}

pub fn build_text(state: &ViewState, now_unix: i64) -> RenderedRow {
    let tone = Tone::from_state(state);
    let Some(snap) = state.cached() else {
        let text = match state {
            ViewState::Loading { .. } => "Codex 用量 · 加载中…",
            ViewState::Error { .. } => "Codex 用量 · 暂不可用",
            ViewState::Ready { .. } | ViewState::Stale { .. } => {
                unreachable!("state has a snapshot")
            }
        };
        return RenderedRow {
            text: text.into(),
            tone,
        };
    };
    let mut segments = snap
        .windows()
        .map(|window| {
            format!(
                "{} {} {}",
                format_window_label(window),
                format_remaining_percent(window),
                format_countdown_compact(window, now_unix)
            )
        })
        .collect::<Vec<_>>();
    if state.stale_marker() {
        segments.push("!".to_string());
    }
    RenderedRow {
        text: segments.join("   "),
        tone,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        RateWindow, UsageSnapshot, ViewState, WindowKind, FIVE_HOUR_SECONDS, WEEKLY_SECONDS,
    };

    fn snap(five: f64, week: f64) -> UsageSnapshot {
        UsageSnapshot {
            five_hour: Some(RateWindow {
                kind: WindowKind::FiveHour,
                used_percent: five,
                window_seconds: FIVE_HOUR_SECONDS,
                resets_at_unix: UsageSnapshot::now() + 3600,
            }),
            weekly: Some(RateWindow {
                kind: WindowKind::Weekly,
                used_percent: week,
                window_seconds: WEEKLY_SECONDS,
                resets_at_unix: UsageSnapshot::now() + 3 * 24 * 3600,
            }),
            fetched_at_unix: UsageSnapshot::now(),
        }
    }

    #[test]
    fn tone_classifies_by_max_used() {
        assert_eq!(
            Tone::from_state(&ViewState::Ready {
                snapshot: snap(40.0, 30.0)
            }),
            Tone::Ok
        );
        assert_eq!(
            Tone::from_state(&ViewState::Ready {
                snapshot: snap(80.0, 20.0)
            }),
            Tone::Warn
        );
        assert_eq!(
            Tone::from_state(&ViewState::Ready {
                snapshot: snap(95.0, 20.0)
            }),
            Tone::Critical
        );
    }

    #[test]
    fn tone_for_stale_state_uses_stale_marker() {
        let s = ViewState::Stale {
            snapshot: snap(40.0, 10.0),
            reason: "network",
        };
        assert_eq!(Tone::from_state(&s), Tone::Stale);
    }

    #[test]
    fn no_snapshot_states_render_status_placeholders() {
        let loading = build_text(&ViewState::Loading { cached: None }, UsageSnapshot::now());
        assert_eq!(loading.text, "Codex 用量 · 加载中…");
        assert_eq!(loading.tone, Tone::Stale);

        let unavailable = build_text(
            &ViewState::Error {
                cached: None,
                reason: "network",
            },
            UsageSnapshot::now(),
        );
        assert_eq!(unavailable.text, "Codex 用量 · 暂不可用");
        assert_eq!(unavailable.tone, Tone::Stale);
    }

    #[test]
    fn weekly_only_row_omits_short_window() {
        let mut snapshot = snap(42.0, 67.0);
        snapshot.five_hour = None;
        let row = build_text(&ViewState::Ready { snapshot }, UsageSnapshot::now());
        assert!(row.text.contains("周限额 33%"));
        assert!(!row.text.contains("5小时"));
        assert!(!row.text.contains("剩余"));
    }

    #[test]
    fn overlay_row_includes_both_segments() {
        let s = ViewState::Ready {
            snapshot: snap(42.0, 67.0),
        };
        let row = build_text(
            &s,
            s.cached()
                .unwrap()
                .five_hour
                .as_ref()
                .unwrap()
                .resets_at_unix
                - 1800,
        );
        assert!(row.text.contains("5小时 58%"));
        assert!(row.text.contains("周限额 33%"));
        assert!(!row.text.contains("剩余"));
    }
}
