//! Formatting for the title-bar overlay and tray tooltip.
//!
//! This module is pure: it never reaches into I/O, GUI, or Win32 APIs and is
//! therefore fully exercised in unit tests. User-visible text shows the
//! remaining percentage by default so the value matches what the user can
//! still use; the technical diagnostic summary continues to render the
//! server-reported `used_percent` so log search and historical triage remain
//! stable. User-facing percentage can be switched to `used_percent` via
//! [`DisplayOptions`].

use crate::localization::{localize_reason, texts};
use crate::model::{RateWindow, ViewState};
use crate::settings::{Language, ResetTimeDisplay, Settings, UsageDisplay};

const ONE_MINUTE: i64 = 60;
const ONE_HOUR: i64 = 60 * ONE_MINUTE;
const ONE_DAY: i64 = 24 * ONE_HOUR;

/// Aggregated user-facing display options derived from [`Settings`]. Formatters
/// pull everything from this struct so callers do not pass raw settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DisplayOptions {
    pub usage: UsageDisplay,
    pub reset_time: ResetTimeDisplay,
    pub language: Language,
}

impl DisplayOptions {
    pub const fn new(
        usage: UsageDisplay,
        reset_time: ResetTimeDisplay,
        language: Language,
    ) -> Self {
        Self {
            usage,
            reset_time,
            language,
        }
    }
}

impl From<&Settings> for DisplayOptions {
    fn from(settings: &Settings) -> Self {
        Self {
            usage: settings.usage_display,
            reset_time: settings.reset_time_display,
            language: settings.language,
        }
    }
}

pub fn format_used_percent(window: &RateWindow) -> String {
    format!("{}%", window.used_percent_rounded())
}

/// Remaining quota value used by the user-facing overlay and tooltip.
/// Mirrors `RateWindow::remaining_percent_rounded` so the value stays
/// consistent with the headline colour thresholds derived from `used_percent`.
pub fn format_remaining_percent(window: &RateWindow) -> String {
    format!("{}%", window.remaining_percent_rounded())
}

/// User-facing percentage respecting the configured [`UsageDisplay`]. Defaults
/// to remaining so existing callers see the same string as before.
pub fn format_usage_percent(window: &RateWindow, usage: UsageDisplay) -> String {
    match usage {
        UsageDisplay::Remaining => format_remaining_percent(window),
        UsageDisplay::Used => format_used_percent(window),
    }
}

/// User-facing description word that pairs with `format_usage_percent`.
pub fn format_usage_label(language: Language, usage: UsageDisplay) -> &'static str {
    match usage {
        UsageDisplay::Remaining => texts(language).usage.remaining,
        UsageDisplay::Used => texts(language).usage.used,
    }
}

/// Human-readable countdown that matches CodexBar-style `1h18m` / `3d4h`.
///
/// The result is intentionally compact because it lives in the title-bar
/// overlay. A wider variant is used for tooltips.
pub fn format_countdown_compact(window: &RateWindow, now_unix: i64, language: Language) -> String {
    let secs = window.seconds_until_reset(now_unix);
    let texts = texts(language);
    if secs == 0 {
        return texts.countdown.now.to_string();
    }
    if language == Language::English {
        return format_english_countdown(secs);
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

fn format_english_countdown(secs: i64) -> String {
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

/// User-facing reset-time string under the chosen display mode. The absolute
/// variant consumes a precomputed local-time shift (seconds east of UTC) so
/// this module stays free of Win32 or process-local side effects.
pub fn format_reset_time(
    window: &RateWindow,
    now_unix: i64,
    mode: ResetTimeDisplay,
    local_shift_seconds: i64,
    language: Language,
) -> String {
    match mode {
        ResetTimeDisplay::Countdown => format_countdown_compact(window, now_unix, language),
        ResetTimeDisplay::Absolute => {
            format_absolute_reset(window.resets_at_unix + local_shift_seconds, language)
        }
    }
}

/// Convert a Unix timestamp into a civil (year, month, day, hour, minute) tuple
/// using Howard Hinnant's `civil_from_days` algorithm. The result is in the
/// caller-supplied local zone; this function never touches the OS clock.
pub fn civil_from_unix(unix: i64) -> (i64, u32, u32, u32, u32) {
    let z = unix / 86_400;
    let secs_of_day = unix.rem_euclid(86_400);
    let hours = (secs_of_day / 3600) as u32;
    let minutes = ((secs_of_day % 3600) / 60) as u32;
    let (year, month, day) = civil_from_days(z);
    (year, month, day, hours, minutes)
}

/// Howard Hinnant's date algorithm. Computes the proleptic Gregorian
/// `(year, month, day)` from a count of days since 1970-01-01. Negative inputs
/// work for pre-1970 timestamps, matching the algorithm's reference behavior.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 {
        (mp + 3) as u32
    } else {
        (mp - 9) as u32
    };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m, d)
}

fn format_absolute_reset(local_unix: i64, language: Language) -> String {
    let (_year, month, day, hour, minute) = civil_from_unix(local_unix);
    let texts = texts(language);
    let month_name = texts.months[(month - 1) as usize];
    if language == Language::English {
        format!("{month_name} {day} {hour:02}:{minute:02}")
    } else {
        format!("{month}月{day}日 {hour:02}:{minute:02}")
    }
}

/// Compact window label used by the user interface without changing the
/// protocol/diagnostic labels stored in the domain model.
pub fn format_window_label(window: &RateWindow, language: Language) -> String {
    use crate::model::WindowKind;

    let labels = texts(language).window;
    match window.kind {
        WindowKind::FiveHour if window.window_seconds == crate::model::FIVE_HOUR_SECONDS => {
            labels.five_hour.to_string()
        }
        WindowKind::Weekly if window.window_seconds == crate::model::WEEKLY_SECONDS => {
            labels.weekly.to_string()
        }
        WindowKind::FiveHour => format!(
            "{}{}",
            labels.short_window_prefix,
            format_duration(window.window_seconds, language)
        ),
        WindowKind::Weekly => format!(
            "{}{}",
            labels.long_window_prefix,
            format_duration(window.window_seconds, language)
        ),
        WindowKind::Other => format!(
            "{}{}",
            labels.other_window_prefix,
            format_duration(window.window_seconds, language)
        ),
    }
}

fn format_duration(seconds: u64, language: Language) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    if language == Language::English {
        return format_english_duration(seconds);
    }
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

fn format_english_duration(seconds: u64) -> String {
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
        format!("{}m", seconds / MINUTE)
    } else {
        format!("{seconds}s")
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
pub fn format_overlay_row(
    state: &ViewState,
    now_unix: i64,
    opts: DisplayOptions,
    local_shift_seconds: i64,
) -> String {
    let Some(snapshot) = state.cached() else {
        return "--".to_string();
    };
    let mut segments = snapshot
        .windows()
        .map(|window| format_window_segment(window, now_unix, opts, local_shift_seconds))
        .collect::<Vec<_>>();
    if state.stale_marker() {
        segments.push("!".to_string());
    }
    segments.join("  ")
}

/// Single window segment shared between the title-bar overlay and tooltip so
/// they never drift apart for the same snapshot.
pub fn format_window_segment(
    window: &RateWindow,
    now_unix: i64,
    opts: DisplayOptions,
    local_shift_seconds: i64,
) -> String {
    format!(
        "{} {} {}",
        format_window_label(window, opts.language),
        format_usage_percent(window, opts.usage),
        format_reset_time(
            window,
            now_unix,
            opts.reset_time,
            local_shift_seconds,
            opts.language,
        ),
    )
}

/// Tooltip-style multi-line string for the tray.
pub fn format_tray_tooltip(
    state: &ViewState,
    now_unix: i64,
    opts: DisplayOptions,
    local_shift_seconds: i64,
) -> String {
    let ui = texts(opts.language);
    let mut lines = Vec::with_capacity(4);
    lines.push("CodexPlus".to_string());
    match state {
        ViewState::Loading { .. } => {
            lines.push(ui.tooltip.loading.to_string());
        }
        ViewState::Ready { snapshot } => {
            lines.extend(
                snapshot
                    .windows()
                    .map(|window| format_usage_line(window, now_unix, opts, local_shift_seconds)),
            );
            lines.push(format!(
                "{}{}",
                ui.tooltip.updated_prefix,
                format_clock(now_unix)
            ));
        }
        ViewState::Stale { snapshot, reason } => {
            lines.extend(
                snapshot
                    .windows()
                    .map(|window| format_usage_line(window, now_unix, opts, local_shift_seconds)),
            );
            lines.push(format!(
                "{}{}{}{}",
                ui.tooltip.stale_prefix,
                localize_reason(opts.language, reason),
                ui.tooltip.stale_suffix,
                format_clock(now_unix),
            ));
        }
        ViewState::Error {
            cached: Some(snapshot),
            reason,
        } => {
            lines.extend(
                snapshot
                    .windows()
                    .map(|window| format_usage_line(window, now_unix, opts, local_shift_seconds)),
            );
            lines.push(format!(
                "{}{}{}{}",
                ui.tooltip.error_prefix,
                localize_reason(opts.language, reason),
                ui.tooltip.error_suffix,
                format_clock(now_unix),
            ));
        }
        ViewState::Error {
            cached: None,
            reason,
        } => {
            lines.push(format!(
                "{}{}",
                ui.tooltip.bare_error_prefix,
                localize_reason(opts.language, reason),
            ));
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

fn format_usage_line(
    window: &RateWindow,
    now_unix: i64,
    opts: DisplayOptions,
    local_shift_seconds: i64,
) -> String {
    let label = format_window_label(window, opts.language);
    let percent = format_usage_percent(window, opts.usage);
    let time = format_reset_time(
        window,
        now_unix,
        opts.reset_time,
        local_shift_seconds,
        opts.language,
    );
    let label_word = format_usage_label(opts.language, opts.usage);
    if opts.language == Language::English {
        format!("{label}: {percent} {label_word}, {time}")
    } else {
        format!("{label}：{label_word} {percent}，{time}")
    }
}

fn usage_line_en(window: &RateWindow, now_unix: i64) -> String {
    let label = window.display_label();
    let used = format_used_percent(window);
    let countdown = format_countdown_en(window, now_unix);
    format!("{label}: {used}, reset in {countdown}")
}

fn format_countdown_en(window: &RateWindow, now_unix: i64) -> String {
    format_english_countdown(window.seconds_until_reset(now_unix))
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
    fn countdown_formats_days_hours_minutes_zh() {
        let w = win(
            WindowKind::Weekly,
            30.0,
            WEEKLY_SECONDS,
            3 * ONE_DAY + 4 * ONE_HOUR,
        );
        assert_eq!(
            format_countdown_compact(&w, 0, Language::SimplifiedChinese),
            "3天4时"
        );
    }

    #[test]
    fn countdown_formats_days_hours_minutes_en() {
        let w = win(
            WindowKind::Weekly,
            30.0,
            WEEKLY_SECONDS,
            3 * ONE_DAY + 4 * ONE_HOUR,
        );
        assert_eq!(format_countdown_compact(&w, 0, Language::English), "3d4h");
    }

    #[test]
    fn countdown_formats_sub_minute_as_one_minute() {
        let w = win(WindowKind::FiveHour, 99.0, FIVE_HOUR_SECONDS, 30);
        assert_eq!(
            format_countdown_compact(&w, 0, Language::SimplifiedChinese),
            "1分"
        );
    }

    #[test]
    fn countdown_past_reset_is_now_zh() {
        let w = win(WindowKind::FiveHour, 100.0, FIVE_HOUR_SECONDS, -10);
        assert_eq!(
            format_countdown_compact(&w, 0, Language::SimplifiedChinese),
            "现在"
        );
    }

    #[test]
    fn countdown_past_reset_is_now_en() {
        let w = win(WindowKind::FiveHour, 100.0, FIVE_HOUR_SECONDS, -10);
        assert_eq!(format_countdown_compact(&w, 0, Language::English), "now");
    }

    #[test]
    fn overlay_row_contains_both_windows_and_stale_marker_zh() {
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
            DisplayOptions::default(),
            0,
        );
        assert!(row.contains("5小时 58% 1时18分"));
        assert!(row.contains("周限额 33% 3天4时"));
        assert!(row.ends_with('!'));
    }

    #[test]
    fn overlay_row_renders_both_windows_en() {
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
        let opts = DisplayOptions::new(
            UsageDisplay::Remaining,
            ResetTimeDisplay::Countdown,
            Language::English,
        );
        let row = format_overlay_row(&ViewState::Ready { snapshot: s }, 0, opts, 0);
        assert!(row.contains("5h 58% 1h18m"));
        assert!(row.contains("Weekly 33% 3d4h"));
    }

    #[test]
    fn only_present_window_is_rendered_zh() {
        let weekly_only = UsageSnapshot {
            five_hour: None,
            weekly: Some(win(WindowKind::Weekly, 67.0, WEEKLY_SECONDS, 3 * ONE_DAY)),
            fetched_at_unix: 0,
        };
        let state = ViewState::Ready {
            snapshot: weekly_only,
        };
        let opts = DisplayOptions::default();
        let row = format_overlay_row(&state, 0, opts, 0);
        let tooltip = format_tray_tooltip(&state, 0, opts, 0);
        assert!(row.contains("周限额 33%"));
        assert!(!row.contains("5小时"));
        assert!(!row.contains("剩余"));
        assert!(tooltip.contains("周限额：剩余 33%"));
        assert!(!tooltip.contains("5小时"));
    }

    #[test]
    fn only_present_window_is_rendered_en() {
        let weekly_only = UsageSnapshot {
            five_hour: None,
            weekly: Some(win(WindowKind::Weekly, 67.0, WEEKLY_SECONDS, 3 * ONE_DAY)),
            fetched_at_unix: 0,
        };
        let state = ViewState::Ready {
            snapshot: weekly_only,
        };
        let opts = DisplayOptions::new(
            UsageDisplay::Remaining,
            ResetTimeDisplay::Countdown,
            Language::English,
        );
        let tooltip = format_tray_tooltip(&state, 0, opts, 0);
        assert!(tooltip.contains("Weekly: 33% remaining"));
    }

    #[test]
    fn remaining_percent_matches_remaining_of_25() {
        let weekly = win(WindowKind::Weekly, 25.0, WEEKLY_SECONDS, 5 * ONE_DAY);
        assert_eq!(format_remaining_percent(&weekly), "75%");
    }

    #[test]
    fn tooltip_summary_shows_remaining() {
        let weekly_only = UsageSnapshot {
            five_hour: None,
            weekly: Some(win(WindowKind::Weekly, 25.0, WEEKLY_SECONDS, 5 * ONE_DAY)),
            fetched_at_unix: 0,
        };
        let state = ViewState::Ready {
            snapshot: weekly_only,
        };
        let tooltip = format_tray_tooltip(&state, 0, DisplayOptions::default(), 0);
        assert!(tooltip.contains("周限额：剩余 75%"));
    }

    #[test]
    fn overlay_row_empty_when_no_cache() {
        let row = format_overlay_row(
            &ViewState::Loading { cached: None },
            0,
            DisplayOptions::default(),
            0,
        );
        assert_eq!(row, "--");
    }

    #[test]
    fn tooltip_includes_chinese_loading_message() {
        let tip = format_tray_tooltip(
            &ViewState::Loading { cached: None },
            0,
            DisplayOptions::default(),
            0,
        );
        assert!(tip.contains("CodexPlus"));
        assert!(tip.contains("正在加载"));
    }

    #[test]
    fn tooltip_includes_chinese_error_reason() {
        let s = snap(
            win(WindowKind::FiveHour, 30.0, FIVE_HOUR_SECONDS, 600),
            win(WindowKind::Weekly, 20.0, WEEKLY_SECONDS, 3600),
        );
        let opts = DisplayOptions::new(
            UsageDisplay::Remaining,
            ResetTimeDisplay::Countdown,
            Language::SimplifiedChinese,
        );
        let tip = format_tray_tooltip(
            &ViewState::Error {
                cached: Some(s),
                reason: "network",
            },
            0,
            opts,
            0,
        );
        assert!(tip.contains("网络异常"));
    }

    #[test]
    fn tooltip_includes_english_error_reason() {
        let s = snap(
            win(WindowKind::FiveHour, 30.0, FIVE_HOUR_SECONDS, 600),
            win(WindowKind::Weekly, 20.0, WEEKLY_SECONDS, 3600),
        );
        let opts = DisplayOptions::new(
            UsageDisplay::Remaining,
            ResetTimeDisplay::Countdown,
            Language::English,
        );
        let tip = format_tray_tooltip(
            &ViewState::Error {
                cached: Some(s),
                reason: "network",
            },
            0,
            opts,
            0,
        );
        assert!(tip.contains("Network unavailable"));
    }

    #[test]
    fn tooltip_includes_english_loading_message() {
        let opts = DisplayOptions::new(
            UsageDisplay::Remaining,
            ResetTimeDisplay::Countdown,
            Language::English,
        );
        let tip = format_tray_tooltip(&ViewState::Loading { cached: None }, 0, opts, 0);
        assert!(tip.contains("Loading Codex usage"));
    }

    #[test]
    fn used_display_renders_used_percent_in_overlay_and_tooltip() {
        let weekly = win(WindowKind::Weekly, 25.0, WEEKLY_SECONDS, 5 * ONE_DAY);
        let snapshot = UsageSnapshot {
            five_hour: None,
            weekly: Some(weekly),
            fetched_at_unix: 0,
        };
        let state = ViewState::Ready { snapshot };
        let opts = DisplayOptions::new(
            UsageDisplay::Used,
            ResetTimeDisplay::Countdown,
            Language::SimplifiedChinese,
        );
        let row = format_overlay_row(&state, 0, opts, 0);
        let tooltip = format_tray_tooltip(&state, 0, opts, 0);
        assert!(row.contains("周限额 25%"));
        assert!(!row.contains("周限额 75%"));
        assert!(tooltip.contains("已用 25%"));
        assert!(
            !tooltip.contains("后重置"),
            "tooltip should drop the 后重置 suffix: {tooltip}"
        );
        assert!(
            !tooltip.contains(" 重置"),
            "tooltip should drop the 重置 suffix: {tooltip}"
        );
    }

    #[test]
    fn used_display_renders_english_overlay_and_tooltip() {
        let weekly = win(WindowKind::Weekly, 25.0, WEEKLY_SECONDS, 5 * ONE_DAY);
        let snapshot = UsageSnapshot {
            five_hour: None,
            weekly: Some(weekly),
            fetched_at_unix: 0,
        };
        let state = ViewState::Ready { snapshot };
        let opts = DisplayOptions::new(
            UsageDisplay::Used,
            ResetTimeDisplay::Countdown,
            Language::English,
        );
        let row = format_overlay_row(&state, 0, opts, 0);
        let tooltip = format_tray_tooltip(&state, 0, opts, 0);
        assert!(row.contains("Weekly 25%"));
        assert!(tooltip.contains("Weekly: 25% used"));
    }

    #[test]
    fn absolute_reset_uses_supplied_local_shift_zh() {
        // Reset time fixed at 2026-07-30 11:34:00 UTC; with a fixed +08:00
        // shift the formatter should render `7月30日 19:34`. The shift is
        // also applied to the embedded numeric for the same `resets_at_unix`
        // value, so the rest of the payload can keep using a far-future
        // timestamp without affecting the assertion.
        let secs = 1_785_411_240_i64; // 2026-07-30 11:34:00 UTC
        let weekly = win(WindowKind::Weekly, 25.0, WEEKLY_SECONDS, secs);
        let snapshot = UsageSnapshot {
            five_hour: None,
            weekly: Some(weekly),
            fetched_at_unix: 0,
        };
        let state = ViewState::Ready { snapshot };
        let opts = DisplayOptions::new(
            UsageDisplay::Remaining,
            ResetTimeDisplay::Absolute,
            Language::SimplifiedChinese,
        );
        let row = format_overlay_row(&state, secs - 3600, opts, 8 * 3600);
        let tooltip = format_tray_tooltip(&state, secs - 3600, opts, 8 * 3600);
        assert!(row.contains("7月30日 19:34"));
        assert!(tooltip.contains("7月30日 19:34"));
        assert!(!tooltip.contains(" 重置"));
    }

    #[test]
    fn absolute_reset_uses_supplied_local_shift_en() {
        let secs = 1_785_411_240_i64; // 2026-07-30 11:34:00 UTC
        let weekly = win(WindowKind::Weekly, 25.0, WEEKLY_SECONDS, secs);
        let snapshot = UsageSnapshot {
            five_hour: None,
            weekly: Some(weekly),
            fetched_at_unix: 0,
        };
        let state = ViewState::Ready { snapshot };
        let opts = DisplayOptions::new(
            UsageDisplay::Remaining,
            ResetTimeDisplay::Absolute,
            Language::English,
        );
        let row = format_overlay_row(&state, secs - 3600, opts, 8 * 3600);
        let tooltip = format_tray_tooltip(&state, secs - 3600, opts, 8 * 3600);
        assert!(row.contains("Jul 30 19:34"));
        assert!(tooltip.contains("Jul 30 19:34"));
    }

    #[test]
    fn absolute_reset_english_single_digit_day_has_no_extra_space() {
        // 2026-07-03 11:34:00 UTC. With +08:00 local shift the absolute
        // formatter should render `Jul 3 19:34` (single-digit day, single
        // space) — not `Jul  3 19:34`.
        let secs = 1_783_078_440_i64; // 2026-07-03 11:34:00 UTC
        let weekly = win(WindowKind::Weekly, 25.0, WEEKLY_SECONDS, secs);
        let snapshot = UsageSnapshot {
            five_hour: None,
            weekly: Some(weekly),
            fetched_at_unix: 0,
        };
        let state = ViewState::Ready { snapshot };
        let opts = DisplayOptions::new(
            UsageDisplay::Remaining,
            ResetTimeDisplay::Absolute,
            Language::English,
        );
        let row = format_overlay_row(&state, secs - 3600, opts, 8 * 3600);
        let tooltip = format_tray_tooltip(&state, secs - 3600, opts, 8 * 3600);
        assert!(row.contains("Jul 3 19:34"));
        assert!(!row.contains("Jul  3"));
        assert!(tooltip.contains("Jul 3 19:34"));
        assert!(!tooltip.contains("Jul  3"));
    }

    #[test]
    fn civil_from_unix_handles_leap_day_and_month_boundaries() {
        // 2024-02-29 12:00:00 UTC == 1_709_208_000
        let (_year, month, day, hour, minute) = civil_from_unix(1_709_208_000);
        assert_eq!((month, day, hour, minute), (2, 29, 12, 0));
        // Month boundary crossing at 2024-03-01 00:00 UTC.
        let (_year, month, day, _hour, _minute) = civil_from_unix(1_709_251_200);
        assert_eq!((month, day), (3, 1));
        // Year boundary 2025-01-01 00:00 UTC == 1_735_689_600
        let (year, month, day, _hour, _minute) = civil_from_unix(1_735_689_600);
        assert_eq!((year, month, day), (2025, 1, 1));
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
        assert_eq!(
            format_window_label(&five, Language::SimplifiedChinese),
            "5小时"
        );
        assert_eq!(
            format_window_label(&week, Language::SimplifiedChinese),
            "周限额"
        );
    }

    #[test]
    fn window_label_picks_canonical_english_names() {
        let five = win(WindowKind::FiveHour, 10.0, FIVE_HOUR_SECONDS, 600);
        let week = win(WindowKind::Weekly, 20.0, WEEKLY_SECONDS, 3600);
        assert_eq!(format_window_label(&five, Language::English), "5h");
        assert_eq!(format_window_label(&week, Language::English), "Weekly");
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
