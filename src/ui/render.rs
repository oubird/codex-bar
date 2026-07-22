//! Pure rendering helpers — the actual GDI paint happens inside
//! [`crate::ui::overlay`]. Keeping the paint decomposition in one place
//! makes it easy to test format mappings and colours without spinning up
//! the Windows runtime.

#![cfg(windows)]

use crate::formatting::{format_countdown_compact, format_used_percent};
use crate::model::ViewState;

/// Logical (DIP) size of the overlay — sized for `5h 42% · 1h18m   week 67% · 3d4h`.
pub const OVERLAY_LOGICAL_WIDTH: i32 = 280;
pub const OVERLAY_LOGICAL_HEIGHT: i32 = 28;

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
        let worst = snap.five_hour.used_percent.max(snap.weekly.used_percent);
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
            ViewState::Loading { .. } => "Codex usage · loading…",
            ViewState::Error { .. } => "Codex usage · unavailable",
            ViewState::Ready { .. } | ViewState::Stale { .. } => {
                unreachable!("state has a snapshot")
            }
        };
        return RenderedRow {
            text: text.into(),
            tone,
        };
    };
    let five_used = format_used_percent(&snap.five_hour);
    let week_used = format_used_percent(&snap.weekly);
    let five_cd = format_countdown_compact(&snap.five_hour, now_unix);
    let week_cd = format_countdown_compact(&snap.weekly, now_unix);
    let five_label = snap.five_hour.display_label();
    let week_label = snap.weekly.display_label();
    let stale = if state.stale_marker() {
        " !".to_string()
    } else {
        String::new()
    };
    RenderedRow {
        text: format!(
            "{five_label} {five_used} {five_cd}   {week_label} {week_used} {week_cd}{stale}"
        ),
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
            five_hour: RateWindow {
                kind: WindowKind::FiveHour,
                used_percent: five,
                window_seconds: FIVE_HOUR_SECONDS,
                resets_at_unix: UsageSnapshot::now() + 3600,
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
        assert_eq!(loading.text, "Codex usage · loading…");
        assert_eq!(loading.tone, Tone::Stale);

        let unavailable = build_text(
            &ViewState::Error {
                cached: None,
                reason: "network",
            },
            UsageSnapshot::now(),
        );
        assert_eq!(unavailable.text, "Codex usage · unavailable");
        assert_eq!(unavailable.tone, Tone::Stale);
    }

    #[test]
    fn overlay_row_includes_both_segments() {
        let s = ViewState::Ready {
            snapshot: snap(42.0, 67.0),
        };
        let row = build_text(&s, s.cached().unwrap().five_hour.resets_at_unix - 1800);
        assert!(row.text.contains("5h 42%"));
        assert!(row.text.contains("week 67%"));
    }
}
