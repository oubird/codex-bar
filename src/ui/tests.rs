//! Cross-module sanity check that the formatting pipeline produces a
//! single line including both windows and respects stale markers.

#![cfg(windows)]

#[cfg(test)]
mod cross_module {
    use crate::formatting::{format_overlay_row, format_tray_tooltip};
    use crate::model::{
        RateWindow, UsageSnapshot, ViewState, WindowKind, FIVE_HOUR_SECONDS, WEEKLY_SECONDS,
    };

    fn snap(five: f64, week: f64, resets_in_five: i64, resets_in_week: i64) -> UsageSnapshot {
        let now = UsageSnapshot::now();
        UsageSnapshot {
            five_hour: Some(RateWindow {
                kind: WindowKind::FiveHour,
                used_percent: five,
                window_seconds: FIVE_HOUR_SECONDS,
                resets_at_unix: now + resets_in_five,
            }),
            weekly: Some(RateWindow {
                kind: WindowKind::Weekly,
                used_percent: week,
                window_seconds: WEEKLY_SECONDS,
                resets_at_unix: now + resets_in_week,
            }),
            fetched_at_unix: now,
        }
    }

    #[test]
    fn overlay_row_and_tooltip_agree_on_stale_marker() {
        let s = snap(40.0, 70.0, 3600, 3 * 24 * 3600);
        let state = ViewState::Stale {
            snapshot: s.clone(),
            reason: "network",
        };
        let now = UsageSnapshot::now();
        let row = format_overlay_row(&state, now);
        let tooltip = format_tray_tooltip(&state, now);
        assert!(row.ends_with('!'));
        assert!(tooltip.contains("数据已过期"));
    }

    #[test]
    fn overlay_row_compact_when_both_windows_present() {
        let s = snap(20.0, 60.0, 5400, 24 * 3600);
        let row = format_overlay_row(
            &ViewState::Ready {
                snapshot: s.clone(),
            },
            UsageSnapshot::now(),
        );
        assert!(row.contains("5小时 20%"));
        assert!(row.contains("每周 60%"));
        assert!(!row.ends_with('!'));
    }
}
