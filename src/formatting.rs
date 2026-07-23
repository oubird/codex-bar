//! Formatting for the title-bar overlay and tray tooltip.
//!
//! This module is pure: it never reaches into I/O, GUI, or Win32 APIs and is
//! therefore fully exercised in unit tests. Display labels intentionally
//! always communicate "used" — never "remaining" — to match Codex's web
//! usage UI and the user's request.

use crate::model::{RateWindow, ViewState};

const ONE_MINUTE: i64 = 60;
const ONE_HOUR: i64 = 60 * ONE_MINUTE;
const ONE_DAY: i64 = 24 * ONE_HOUR;

pub fn format_used_percent(window: &RateWindow) -> String {
    format!("{}%", window.used_percent_rounded())
}

/// Human-readable countdown that matches CodexBar-style `1h18m` / `3d4h`.
///
/// The result is intentionally compact because it lives in the title-bar
/// overlay. A wider variant is used for tooltips.
pub fn format_countdown_compact(window: &RateWindow, now_unix: i64) -> String {
    let secs = window.seconds_until_reset(now_unix);
    if secs == 0 {
        return "现在".to_string();
    }
    if secs >= ONE_DAY {
        let days = secs / ONE_DAY;
        let hours = (secs % ONE_DAY) / ONE_HOUR;
        if hours == 0 {
            format!("{days}天")
        } else {
            format!("{days}天{hours}时")
        }
    } else if secs >= ONE_HOUR {
        let hours = secs / ONE_HOUR;
        let minutes = (secs % ONE_HOUR) / ONE_MINUTE;
        if minutes == 0 {
            format!("{hours}时")
        } else {
            format!("{hours}时{minutes}分")
        }
    } else {
        let minutes = ((secs + 30) / ONE_MINUTE).max(1);
        format!("{minutes}分")
    }
}

/// Compact Chinese label used by the user interface without changing the
/// protocol/diagnostic labels stored in the domain model.
pub fn format_window_label(window: &RateWindow) -> String {
    use crate::model::WindowKind;

    match window.kind {
        WindowKind::FiveHour if window.window_seconds == crate::model::FIVE_HOUR_SECONDS => {
            "5小时".to_string()
        }
        WindowKind::Weekly if window.window_seconds == crate::model::WEEKLY_SECONDS => {
            "每周".to_string()
        }
        WindowKind::FiveHour => format!("短周期 {}", format_duration_zh(window.window_seconds)),
        WindowKind::Weekly => format!("长周期 {}", format_duration_zh(window.window_seconds)),
        WindowKind::Other => format!("其他 {}", format_duration_zh(window.window_seconds)),
    }
}

fn format_duration_zh(seconds: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    if seconds >= DAY {
        let days = seconds / DAY;
        let hours = (seconds % DAY) / HOUR;
        if hours == 0 {
            format!("{days}天")
        } else {
            format!("{days}天{hours}时")
        }
    } else if seconds >= HOUR {
        let hours = seconds / HOUR;
        let minutes = (seconds % HOUR) / MINUTE;
        if minutes == 0 {
            format!("{hours}时")
        } else {
            format!("{hours}时{minutes}分")
        }
    } else if seconds >= MINUTE {
        format!("{}分", seconds / MINUTE)
    } else {
        format!("{seconds}秒")
    }
}

/// Seconds until the visible countdown text changes, for scheduling paint.
pub fn seconds_until_countdown_change(window: &RateWindow, now_unix: i64) -> i64 {
    let total = (window.resets_at_unix - now_unix).max(0);
    if total >= ONE_DAY {
        let rem = total % ONE_DAY;
        ONE_DAY - rem.min(ONE_DAY - 1)
    } else if total >= ONE_HOUR {
        let rem = total % ONE_HOUR;
        ONE_HOUR - rem.min(ONE_HOUR - 1)
    } else if total >= ONE_MINUTE {
        let rem = total % ONE_MINUTE;
        ONE_MINUTE - rem.min(ONE_MINUTE - 1)
    } else if total > 0 {
        total
    } else {
        1
    }
}

/// Compact, single-row string used in the title-bar overlay.
pub fn format_overlay_row(state: &ViewState, now_unix: i64) -> String {
    let Some(snapshot) = state.cached() else {
        return "--".to_string();
    };
    let mut segments = snapshot
        .windows()
        .map(|window| {
            format!(
                "{} {} {}",
                format_window_label(window),
                format_used_percent(window),
                format_countdown_compact(window, now_unix)
            )
        })
        .collect::<Vec<_>>();
    if state.stale_marker() {
        segments.push("!".to_string());
    }
    segments.join("  ")
}

/// Tooltip-style multi-line string for the tray.
pub fn format_tray_tooltip(state: &ViewState, now_unix: i64) -> String {
    let mut lines = Vec::with_capacity(4);
    lines.push("CodexPlus".to_string());
    match state {
        ViewState::Loading { .. } => {
            lines.push("正在加载 Codex 用量…".to_string());
        }
        ViewState::Ready { snapshot } => {
            lines.extend(
                snapshot
                    .windows()
                    .map(|window| usage_line_zh(window, now_unix)),
            );
            lines.push(format!("更新于 {}", format_clock(now_unix)));
        }
        ViewState::Stale { snapshot, reason } => {
            lines.extend(
                snapshot
                    .windows()
                    .map(|window| usage_line_zh(window, now_unix)),
            );
            lines.push(format!(
                "数据已过期（{}）— 更新于 {}",
                localize_reason(reason),
                format_clock(now_unix)
            ));
        }
        ViewState::Error {
            cached: Some(snapshot),
            reason,
        } => {
            lines.extend(
                snapshot
                    .windows()
                    .map(|window| usage_line_zh(window, now_unix)),
            );
            lines.push(format!(
                "发生错误（{}）— 更新于 {}",
                localize_reason(reason),
                format_clock(now_unix)
            ));
        }
        ViewState::Error {
            cached: None,
            reason,
        } => {
            lines.push(format!("错误：{}", localize_reason(reason)));
        }
    }
    let tooltip = lines.join("\n");
    truncate_utf16(&tooltip, 511)
}

/// English state summary used only inside the technical diagnostic report.
pub fn format_diagnostic_summary(state: &ViewState, now_unix: i64) -> String {
    let mut lines = Vec::with_capacity(4);
    lines.push("CodexPlus".to_string());
    match state {
        ViewState::Loading { .. } => lines.push("Loading Codex usage...".to_string()),
        ViewState::Ready { snapshot } => {
            lines.extend(
                snapshot
                    .windows()
                    .map(|window| usage_line_en(window, now_unix)),
            );
            lines.push(format!("Updated {}", format_clock(now_unix)));
        }
        ViewState::Stale { snapshot, reason } => {
            lines.extend(
                snapshot
                    .windows()
                    .map(|window| usage_line_en(window, now_unix)),
            );
            lines.push(format!(
                "Stale ({reason}) — updated {}",
                format_clock(now_unix)
            ));
        }
        ViewState::Error {
            cached: Some(snapshot),
            reason,
        } => {
            lines.extend(
                snapshot
                    .windows()
                    .map(|window| usage_line_en(window, now_unix)),
            );
            lines.push(format!(
                "Error ({reason}) — updated {}",
                format_clock(now_unix)
            ));
        }
        ViewState::Error {
            cached: None,
            reason,
        } => lines.push(format!("Error: {reason}")),
    }
    truncate_utf16(&lines.join("\n"), 511)
}

fn usage_line_zh(window: &RateWindow, now_unix: i64) -> String {
    let label = format_window_label(window);
    let used = format_used_percent(window);
    let countdown = format_countdown_compact(window, now_unix);
    format!("{label}：已用 {used}，{countdown}后重置")
}

fn usage_line_en(window: &RateWindow, now_unix: i64) -> String {
    let label = window.display_label();
    let used = format_used_percent(window);
    let countdown = format_countdown_en(window, now_unix);
    format!("{label}: {used}, reset in {countdown}")
}

fn format_countdown_en(window: &RateWindow, now_unix: i64) -> String {
    let secs = window.seconds_until_reset(now_unix);
    if secs == 0 {
        return "now".to_string();
    }
    if secs >= ONE_DAY {
        let days = secs / ONE_DAY;
        let hours = (secs % ONE_DAY) / ONE_HOUR;
        if hours == 0 {
            format!("{days}d")
        } else {
            format!("{days}d{hours}h")
        }
    } else if secs >= ONE_HOUR {
        let hours = secs / ONE_HOUR;
        let minutes = (secs % ONE_HOUR) / ONE_MINUTE;
        if minutes == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h{minutes}m")
        }
    } else {
        let minutes = ((secs + 30) / ONE_MINUTE).max(1);
        format!("{minutes}m")
    }
}

fn localize_reason(reason: &str) -> &str {
    match reason {
        "auth" => "认证失败",
        "network" => "网络异常",
        "proxy" => "代理异常",
        "rate limited" => "请求受限",
        "protocol" => "服务协议变化",
        "error" => "未知错误",
        _ => "未知错误",
    }
}

/// `%H:%M` local-time clock without pulling in chrono.
pub fn format_clock(unix: i64) -> String {
    let secs_in_day = 24 * 60 * 60;
    let secs_of_day = unix.rem_euclid(secs_in_day);
    let hours = (secs_of_day / 3600) % 24;
    let minutes = (secs_of_day / 60) % 60;
    format!("{hours:02}:{minutes:02}")
}

/// Windows `NotifyIcon.Text` is documented as supporting 128 TCHARs, but in
/// practice the shell silently truncates around 511 UTF-16 code units.
fn truncate_utf16(value: &str, max_code_units: usize) -> String {
    let mut buf = String::new();
    let mut units = 0usize;
    for ch in value.chars() {
        let cpw = ch.len_utf16();
        if units + cpw > max_code_units {
            break;
        }
        buf.push(ch);
        units += cpw;
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        RateWindow, UsageSnapshot, ViewState, WindowKind, FIVE_HOUR_SECONDS, WEEKLY_SECONDS,
    };

    fn win(kind: WindowKind, used: f64, seconds: u64, resets_in: i64) -> RateWindow {
        RateWindow {
            kind,
            used_percent: used,
            window_seconds: seconds,
            resets_at_unix: resets_in,
        }
    }

    fn snap(five: RateWindow, week: RateWindow) -> UsageSnapshot {
        UsageSnapshot {
            five_hour: Some(five),
            weekly: Some(week),
            fetched_at_unix: 0,
        }
    }

    #[test]
    fn countdown_formats_days_hours_minutes() {
        let w = win(
            WindowKind::Weekly,
            30.0,
            WEEKLY_SECONDS,
            3 * ONE_DAY + 4 * ONE_HOUR,
        );
        assert_eq!(format_countdown_compact(&w, 0), "3天4时");
    }

    #[test]
    fn countdown_formats_sub_minute_as_one_minute() {
        let w = win(WindowKind::FiveHour, 99.0, FIVE_HOUR_SECONDS, 30);
        assert_eq!(format_countdown_compact(&w, 0), "1分");
    }

    #[test]
    fn countdown_past_reset_is_now() {
        let w = win(WindowKind::FiveHour, 100.0, FIVE_HOUR_SECONDS, -10);
        assert_eq!(format_countdown_compact(&w, 0), "现在");
    }

    #[test]
    fn overlay_row_contains_both_windows_and_stale_marker() {
        let s = snap(
            win(
                WindowKind::FiveHour,
                42.0,
                FIVE_HOUR_SECONDS,
                ONE_HOUR + 18 * ONE_MINUTE,
            ),
            win(
                WindowKind::Weekly,
                67.0,
                WEEKLY_SECONDS,
                3 * ONE_DAY + 4 * ONE_HOUR,
            ),
        );
        let row = format_overlay_row(
            &ViewState::Stale {
                snapshot: s,
                reason: "network",
            },
            0,
        );
        assert!(row.contains("5小时 42% 1时18分"));
        assert!(row.contains("每周 67% 3天4时"));
        assert!(row.ends_with('!'));
    }

    #[test]
    fn only_present_window_is_rendered() {
        let weekly_only = UsageSnapshot {
            five_hour: None,
            weekly: Some(win(WindowKind::Weekly, 67.0, WEEKLY_SECONDS, 3 * ONE_DAY)),
            fetched_at_unix: 0,
        };
        let state = ViewState::Ready {
            snapshot: weekly_only,
        };
        let row = format_overlay_row(&state, 0);
        let tooltip = format_tray_tooltip(&state, 0);
        assert!(row.contains("每周 67%"));
        assert!(!row.contains("5小时"));
        assert!(tooltip.contains("每周：已用 67%"));
        assert!(!tooltip.contains("5小时"));
    }

    #[test]
    fn overlay_row_empty_when_no_cache() {
        let row = format_overlay_row(&ViewState::Loading { cached: None }, 0);
        assert_eq!(row, "--");
    }

    #[test]
    fn tooltip_includes_chinese_loading_message() {
        let tip = format_tray_tooltip(&ViewState::Loading { cached: None }, 0);
        assert!(tip.contains("CodexPlus"));
        assert!(tip.contains("正在加载"));
    }

    #[test]
    fn tooltip_includes_chinese_error_reason() {
        let s = snap(
            win(WindowKind::FiveHour, 30.0, FIVE_HOUR_SECONDS, 600),
            win(WindowKind::Weekly, 20.0, WEEKLY_SECONDS, 3600),
        );
        let tip = format_tray_tooltip(
            &ViewState::Error {
                cached: Some(s),
                reason: "network",
            },
            0,
        );
        assert!(tip.contains("网络异常"));
    }

    #[test]
    fn diagnostic_summary_keeps_english_for_tech_report() {
        let s = snap(
            win(WindowKind::FiveHour, 30.0, FIVE_HOUR_SECONDS, 600),
            win(WindowKind::Weekly, 20.0, WEEKLY_SECONDS, 3600),
        );
        let summary = format_diagnostic_summary(
            &ViewState::Stale {
                snapshot: s,
                reason: "network",
            },
            0,
        );
        assert!(summary.contains("Stale (network)"));
    }

    #[test]
    fn window_label_picks_canonical_chinese_names() {
        let five = win(WindowKind::FiveHour, 10.0, FIVE_HOUR_SECONDS, 600);
        let week = win(WindowKind::Weekly, 20.0, WEEKLY_SECONDS, 3600);
        assert_eq!(format_window_label(&five), "5小时");
        assert_eq!(format_window_label(&week), "每周");
    }

    #[test]
    fn seconds_until_countdown_change_aligns_to_unit() {
        let w = win(
            WindowKind::FiveHour,
            50.0,
            FIVE_HOUR_SECONDS,
            3 * ONE_HOUR + 30,
        );
        let change = seconds_until_countdown_change(&w, 3 * ONE_HOUR);
        // remaining seconds == 30, which is below ONE_MINUTE, so the next
        // change is in those 30 seconds.
        assert_eq!(change, 30);
    }

    #[test]
    fn tooltip_truncates_without_splitting_surrogate() {
        let emoji = "🎉"; // surrogate pair in UTF-16
        let input = emoji.repeat(40);
        let truncated = truncate_utf16(&input, 4);
        assert!(truncated.chars().count() <= 2);
    }
}
