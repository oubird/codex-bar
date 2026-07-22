//! Domain models for CodexPlus.
//!
//! These types are deliberately decoupled from the upstream JSON shape so
//! that, if `wham/usage` renames fields or adds windows, only the provider
//! module has to change. The overlay never reads `primary_window` /
//! `secondary_window` directly.

use std::time::{SystemTime, UNIX_EPOCH};

/// Required Codex usage windows for this MVP.
///
/// We intentionally only recognise the two limit windows the Codex web UI
/// currently surfaces as "5 hour" and "weekly". Anything else is folded into
/// a typed `Other` and surfaced through diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    FiveHour,
    Weekly,
    Other,
}

impl WindowKind {
    /// Canonical label used in compact UI text and diagnostics.
    pub fn label(&self) -> &'static str {
        match self {
            WindowKind::FiveHour => "5h",
            WindowKind::Weekly => "week",
            WindowKind::Other => "extra",
        }
    }
}

pub const FIVE_HOUR_SECONDS: u64 = 18_000;
pub const WEEKLY_SECONDS: u64 = 604_800;
pub const MIN_WINDOW_SECONDS: u64 = 60;
pub const MAX_WINDOW_SECONDS: u64 = 60 * 60 * 24 * 31; // one month
pub const CLOCK_SKEW_SECONDS: i64 = 300; // tolerate 5 minutes of drift

/// Inclusive lower bound (in seconds) for the *primary* codex window slot.
pub const PRIMARY_WINDOW_MIN_SECONDS: u64 = 60 * 60; // 1 hour
/// Inclusive upper bound (in seconds) for the *primary* codex window slot.
pub const PRIMARY_WINDOW_MAX_SECONDS: u64 = 24 * 60 * 60; // 24 hours
/// Inclusive lower bound (in seconds) for the *secondary* codex window slot.
pub const SECONDARY_WINDOW_MIN_SECONDS: u64 = 24 * 60 * 60; // 24 hours
/// Inclusive upper bound (in seconds) for the *secondary* codex window slot.
pub const SECONDARY_WINDOW_MAX_SECONDS: u64 = MAX_WINDOW_SECONDS;

impl WindowKind {
    /// Canonical CodexPlus display duration for this slot. `None` when the
    /// role does not map to a fixed display length; the renderer falls back
    /// to a dynamic label in that case.
    pub fn canonical_seconds(self) -> Option<u64> {
        match self {
            WindowKind::FiveHour => Some(FIVE_HOUR_SECONDS),
            WindowKind::Weekly => Some(WEEKLY_SECONDS),
            WindowKind::Other => None,
        }
    }

    /// Inclusive `(min, max)` range the role accepts in seconds.
    pub fn duration_bounds(self) -> Option<(u64, u64)> {
        match self {
            WindowKind::FiveHour => Some((PRIMARY_WINDOW_MIN_SECONDS, PRIMARY_WINDOW_MAX_SECONDS)),
            WindowKind::Weekly => {
                Some((SECONDARY_WINDOW_MIN_SECONDS, SECONDARY_WINDOW_MAX_SECONDS))
            }
            WindowKind::Other => None,
        }
    }

    /// Whether a server-supplied duration fits the role's safety bounds.
    pub fn accepts_duration(self, seconds: u64) -> bool {
        self.duration_bounds()
            .is_some_and(|(min, max)| (min..=max).contains(&seconds))
    }

    /// Short, role-specific label used in UI/diagnostics when no canonical
    /// duration matches.
    pub fn role_label(self) -> &'static str {
        match self {
            WindowKind::FiveHour => "primary",
            WindowKind::Weekly => "secondary",
            WindowKind::Other => "other",
        }
    }
}

/// A normalised rate-limit window.
///
/// `kind` denotes the *role* assigned by the upstream JSON layout
/// (`primary_window` / `secondary_window`), not the duration itself.
/// `window_seconds` is the duration reported by the server and validated
/// against [`WindowKind::accepts_duration`].
#[derive(Debug, Clone, PartialEq)]
pub struct RateWindow {
    pub kind: WindowKind,
    pub used_percent: f64,
    pub window_seconds: u64,
    /// Absolute reset timestamp in Unix seconds.
    pub resets_at_unix: i64,
}

impl RateWindow {
    pub fn used_percent_rounded(&self) -> i64 {
        self.used_percent.round() as i64
    }

    pub fn remaining_percent_rounded(&self) -> i64 {
        (100.0 - self.used_percent).round() as i64
    }

    /// Seconds until the window resets, clamped to at least 0.
    pub fn seconds_until_reset(&self, now_unix: i64) -> i64 {
        (self.resets_at_unix - now_unix).max(0)
    }

    /// Compact label for UI / tooltip text. Matches the canonical Codex
    /// duration when one exists, otherwise renders the role plus a compact
    /// representation of the actual duration.
    pub fn display_label(&self) -> String {
        if let Some(canonical) = self.kind.canonical_seconds() {
            if self.window_seconds == canonical {
                return self.kind.label().to_string();
            }
        }
        format!(
            "{} {}",
            self.kind.role_label(),
            format_duration_compact(self.window_seconds)
        )
    }
}

/// Format a number of seconds as `90m`, `5h`, `5h30m`, `8d`, `8d4h`.
fn format_duration_compact(seconds: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    if seconds >= DAY {
        let days = seconds / DAY;
        let hours = (seconds % DAY) / HOUR;
        if hours == 0 {
            format!("{days}d")
        } else {
            format!("{days}d{hours}h")
        }
    } else if seconds >= HOUR {
        let hours = seconds / HOUR;
        let minutes = (seconds % HOUR) / MINUTE;
        if minutes == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h{minutes}m")
        }
    } else if seconds >= MINUTE {
        let minutes = seconds / MINUTE;
        format!("{minutes}m")
    } else {
        format!("{seconds}s")
    }
}

/// A pair of validated Codex usage windows.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageSnapshot {
    pub five_hour: RateWindow,
    pub weekly: RateWindow,
    /// When we fetched the snapshot, in Unix seconds.
    pub fetched_at_unix: i64,
}

impl UsageSnapshot {
    pub fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// Both slots exist, contain valid percentages, fit their role bounds,
    /// maintain a strictly smaller primary than secondary window, and have
    /// reset timestamps anchored against the *actual* reported duration.
    pub fn is_sane(&self) -> bool {
        let now = self.fetched_at_unix;
        let five = &self.five_hour;
        let week = &self.weekly;
        if five.kind != WindowKind::FiveHour || week.kind != WindowKind::Weekly {
            return false;
        }
        if !five.used_percent.is_finite() || !week.used_percent.is_finite() {
            return false;
        }
        if !(0.0..=100.0).contains(&five.used_percent)
            || !(0.0..=100.0).contains(&week.used_percent)
        {
            return false;
        }
        if !WindowKind::FiveHour.accepts_duration(five.window_seconds)
            || !WindowKind::Weekly.accepts_duration(week.window_seconds)
        {
            return false;
        }
        if five.window_seconds >= week.window_seconds {
            return false;
        }
        if !reset_within_bounds(five.resets_at_unix, five.window_seconds, now)
            || !reset_within_bounds(week.resets_at_unix, week.window_seconds, now)
        {
            return false;
        }
        true
    }
}

fn reset_within_bounds(reset_at: i64, window_seconds: u64, now: i64) -> bool {
    let lower = now - CLOCK_SKEW_SECONDS;
    let upper = now + (window_seconds as i64) + CLOCK_SKEW_SECONDS;
    reset_at >= lower && reset_at <= upper
}

/// What the user should see right now.
///
/// `Loading` and `Stale` may carry an optional last-good snapshot so the
/// overlay can render historical numbers with an explicit stale marker.
#[derive(Debug, Clone)]
pub enum ViewState {
    Loading {
        cached: Option<UsageSnapshot>,
    },
    Ready {
        snapshot: UsageSnapshot,
    },
    Stale {
        snapshot: UsageSnapshot,
        reason: &'static str,
    },
    Error {
        cached: Option<UsageSnapshot>,
        reason: &'static str,
    },
}

impl ViewState {
    pub fn cached(&self) -> Option<&UsageSnapshot> {
        match self {
            ViewState::Loading { cached } | ViewState::Error { cached, .. } => cached.as_ref(),
            ViewState::Ready { snapshot } | ViewState::Stale { snapshot, .. } => Some(snapshot),
        }
    }

    /// Every state has a compact overlay representation. A state without a
    /// snapshot renders a status placeholder instead of invented quota data.
    pub fn is_displayable(&self) -> bool {
        true
    }

    pub fn stale_marker(&self) -> bool {
        matches!(
            self,
            ViewState::Stale { .. }
                | ViewState::Error {
                    cached: Some(_),
                    ..
                }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap_with(five: (f64, i64), week: (f64, i64)) -> UsageSnapshot {
        UsageSnapshot {
            five_hour: RateWindow {
                kind: WindowKind::FiveHour,
                used_percent: five.0,
                window_seconds: FIVE_HOUR_SECONDS,
                resets_at_unix: five.1,
            },
            weekly: RateWindow {
                kind: WindowKind::Weekly,
                used_percent: week.0,
                window_seconds: WEEKLY_SECONDS,
                resets_at_unix: week.1,
            },
            fetched_at_unix: UsageSnapshot::now(),
        }
    }

    #[test]
    fn sane_snapshot_passes_basic_validation() {
        let now = UsageSnapshot::now();
        let snap = UsageSnapshot {
            five_hour: RateWindow {
                kind: WindowKind::FiveHour,
                used_percent: 50.0,
                window_seconds: FIVE_HOUR_SECONDS,
                resets_at_unix: now + 3600,
            },
            weekly: RateWindow {
                kind: WindowKind::Weekly,
                used_percent: 10.0,
                window_seconds: WEEKLY_SECONDS,
                resets_at_unix: now + 24 * 3600,
            },
            fetched_at_unix: now,
        };
        assert!(snap.is_sane());
    }

    #[test]
    fn insane_snapshot_fails_when_window_does_not_match() {
        let mut snap = snap_with((50.0, 0), (10.0, 0));
        snap.five_hour.window_seconds = 60; // below PRIMARY_WINDOW_MIN_SECONDS
        assert!(!snap.is_sane());
    }

    #[test]
    fn sane_snapshot_accepts_dynamic_durations() {
        let now = UsageSnapshot::now();
        let snap = UsageSnapshot {
            five_hour: RateWindow {
                kind: WindowKind::FiveHour,
                used_percent: 25.0,
                window_seconds: 6 * 3600,
                resets_at_unix: now + 6 * 3600,
            },
            weekly: RateWindow {
                kind: WindowKind::Weekly,
                used_percent: 40.0,
                window_seconds: 8 * 86_400,
                resets_at_unix: now + 8 * 86_400,
            },
            fetched_at_unix: now,
        };
        assert!(snap.is_sane());
    }

    #[test]
    fn insane_snapshot_rejects_inverted_durations() {
        let now = UsageSnapshot::now();
        let snap = UsageSnapshot {
            five_hour: RateWindow {
                kind: WindowKind::FiveHour,
                used_percent: 25.0,
                window_seconds: 9 * 86_400,
                resets_at_unix: now + 9 * 86_400,
            },
            weekly: RateWindow {
                kind: WindowKind::Weekly,
                used_percent: 40.0,
                window_seconds: WEEKLY_SECONDS,
                resets_at_unix: now + WEEKLY_SECONDS as i64,
            },
            fetched_at_unix: now,
        };
        assert!(!snap.is_sane());
    }

    #[test]
    fn insane_snapshot_rejects_reset_beyond_actual_duration() {
        let now = UsageSnapshot::now();
        let snap = UsageSnapshot {
            five_hour: RateWindow {
                kind: WindowKind::FiveHour,
                used_percent: 25.0,
                window_seconds: 6 * 3600,
                resets_at_unix: now + 24 * 3600,
            },
            weekly: RateWindow {
                kind: WindowKind::Weekly,
                used_percent: 40.0,
                window_seconds: WEEKLY_SECONDS,
                resets_at_unix: now + WEEKLY_SECONDS as i64,
            },
            fetched_at_unix: now,
        };
        assert!(!snap.is_sane());
    }

    #[test]
    fn display_label_uses_canonical_when_duration_matches() {
        let w = RateWindow {
            kind: WindowKind::FiveHour,
            used_percent: 10.0,
            window_seconds: FIVE_HOUR_SECONDS,
            resets_at_unix: 0,
        };
        assert_eq!(w.display_label(), "5h");
    }

    #[test]
    fn display_label_falls_back_to_role_and_duration_when_non_canonical() {
        let w = RateWindow {
            kind: WindowKind::FiveHour,
            used_percent: 10.0,
            window_seconds: 6 * 3600,
            resets_at_unix: 0,
        };
        assert_eq!(w.display_label(), "primary 6h");
    }

    #[test]
    fn window_kind_accepts_dynamic_primary_and_secondary() {
        assert!(WindowKind::FiveHour.accepts_duration(6 * 3600));
        assert!(!WindowKind::FiveHour.accepts_duration(36 * 3600));
        assert!(WindowKind::Weekly.accepts_duration(8 * 86_400));
        assert!(!WindowKind::Weekly.accepts_duration(12 * 3600));
    }

    #[test]
    fn view_state_carries_last_good_through_errors() {
        let snap = snap_with((50.0, 0), (10.0, 0));
        let state = ViewState::Error {
            cached: Some(snap.clone()),
            reason: "network",
        };
        assert!(state.cached().is_some());
        assert!(state.is_displayable());
        assert!(state.stale_marker());
    }

    #[test]
    fn view_state_error_without_cache_renders_a_placeholder() {
        let state = ViewState::Error {
            cached: None,
            reason: "no auth",
        };
        assert!(state.is_displayable());
        assert!(state.cached().is_none());
        assert!(!state.stale_marker());
    }
}
