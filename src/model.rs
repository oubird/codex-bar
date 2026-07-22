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

/// A normalised rate-limit window.
///
/// Percentages are kept as `f64` internally so that the renderer can
/// display rounded values, but they are validated to be finite and inside
/// `[0, 100]` before reaching this type — see [`crate::provider`].
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

    /// Both targets exist and are within tolerance.
    pub fn is_sane(&self) -> bool {
        let now = self.fetched_at_unix;
        self.five_hour.window_seconds == FIVE_HOUR_SECONDS
            && self.weekly.window_seconds == WEEKLY_SECONDS
            && (self.five_hour.resets_at_unix - now).abs()
                <= self.five_hour.window_seconds as i64 + CLOCK_SKEW_SECONDS
            && (self.weekly.resets_at_unix - now).abs()
                <= self.weekly.window_seconds as i64 + CLOCK_SKEW_SECONDS
            && self.five_hour.used_percent.is_finite()
            && self.weekly.used_percent.is_finite()
            && (0.0..=100.0).contains(&self.five_hour.used_percent)
            && (0.0..=100.0).contains(&self.weekly.used_percent)
    }
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

    pub fn is_displayable(&self) -> bool {
        !matches!(self, ViewState::Error { cached: None, .. })
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
        snap.five_hour.window_seconds = 60;
        assert!(!snap.is_sane());
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
    fn view_state_error_without_cache_is_not_displayable() {
        let state = ViewState::Error {
            cached: None,
            reason: "no auth",
        };
        assert!(!state.is_displayable());
        assert!(!state.stale_marker());
    }
}
