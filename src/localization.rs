//! Static UI copy for both supported languages.
//!
//! CodexPlus ships exactly two UI locales: `SimplifiedChinese` (default,
//! matching earlier releases) and `English`. Dynamic sentences that include
//! numeric formatting live in `formatting`; this module only owns literal
//! strings and stable reason-key lookup tables so Win32 UI code paths can
//! stop hard-coding duplicated constants. No third-party i18n dependency is
//! introduced; both tables compile down to plain `&'static str` slices.

use crate::settings::Language;

/// All user-visible literal text the application renders, split by active
/// language. Two intentional design choices:
///
/// * Strings stay terse so the title-bar overlay can fit both languages
///   inside the existing logical width.
/// * Diagnostic / log messages intentionally stay in English; they are not
///   addressed at end users and keeping them stable makes log search useful.
#[derive(Debug, Clone, Copy)]
pub struct ReasonLabel {
    pub auth: &'static str,
    pub network: &'static str,
    pub proxy: &'static str,
    pub rate_limited: &'static str,
    pub protocol: &'static str,
    pub error: &'static str,
}

// Maps the internal reason-key set produced by `AppState::reason_for` to a
// user-facing label. The internal key set is fixed so it produces the same
// stable identifiers across UI languages.

#[derive(Debug, Clone, Copy)]
pub struct OverlayStatus {
    pub loading: &'static str,
    pub unavailable: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct TooltipStatus {
    pub loading: &'static str,
    pub updated_prefix: &'static str,
    pub stale_prefix: &'static str,
    pub stale_suffix: &'static str,
    pub error_prefix: &'static str,
    pub error_suffix: &'static str,
    pub bare_error_prefix: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct WindowLabel {
    pub five_hour: &'static str,
    pub weekly: &'static str,
    pub short_window_prefix: &'static str,
    pub long_window_prefix: &'static str,
    pub other_window_prefix: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct Countdown {
    pub now: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct TrayMenu {
    pub summary_loading: &'static str,
    pub refresh: &'static str,
    pub toggle_overlay: &'static str,
    pub settings: &'static str,
    pub diagnose: &'static str,
    pub exit: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct UsageWord {
    pub remaining: &'static str,
    pub used: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct SettingsWindow {
    pub title: &'static str,
    pub page_title: &'static str,
    pub page_subtitle: &'static str,
    pub display_card_title: &'static str,
    pub language_label: &'static str,
    pub language_zh_cn: &'static str,
    pub language_english: &'static str,
    pub overlay_checkbox: &'static str,
    pub refresh_label: &'static str,
    pub usage_label: &'static str,
    pub usage_remaining: &'static str,
    pub usage_used: &'static str,
    pub reset_label: &'static str,
    pub reset_countdown: &'static str,
    pub reset_absolute: &'static str,
    pub quick_prompts_card_title: &'static str,
    pub quick_prompts_help: &'static str,
    pub quick_prompts_invalid: &'static str,
    pub proxy_card_title: &'static str,
    pub proxy_env: &'static str,
    pub proxy_direct: &'static str,
    pub proxy_custom: &'static str,
    pub proxy_address_label: &'static str,
    pub proxy_help: &'static str,
    pub cancel: &'static str,
    pub save: &'static str,
    pub invalid_proxy_message: &'static str,
    pub invalid_proxy_title: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct DiagnoseDialog {
    pub title: &'static str,
    pub status_ok: &'static str,
    pub status_failed_prefix: &'static str,
    pub log_label: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct UiTexts {
    pub language_name: &'static str,
    pub overlay: OverlayStatus,
    pub tooltip: TooltipStatus,
    pub window: WindowLabel,
    pub countdown: Countdown,
    pub tray: TrayMenu,
    pub usage: UsageWord,
    pub settings: SettingsWindow,
    pub diagnose: DiagnoseDialog,
    pub reason: ReasonLabel,
    /// Short month names used for `Jul 30 19:34` style reset timestamps.
    pub months: [&'static str; 12],
}

const ZH_TEXTS: UiTexts = UiTexts {
    language_name: "简体中文",
    overlay: OverlayStatus {
        loading: "Codex 用量 · 加载中…",
        unavailable: "Codex 用量 · 暂不可用",
    },
    tooltip: TooltipStatus {
        loading: "正在加载 Codex 用量…",
        updated_prefix: "更新于 ",
        stale_prefix: "数据已过期（",
        stale_suffix: "）— 更新于 ",
        error_prefix: "发生错误（",
        error_suffix: "）— 更新于 ",
        bare_error_prefix: "错误：",
    },
    window: WindowLabel {
        five_hour: "5小时",
        weekly: "周限额",
        short_window_prefix: "短周期 ",
        long_window_prefix: "长周期 ",
        other_window_prefix: "其他 ",
    },
    countdown: Countdown { now: "现在" },
    tray: TrayMenu {
        summary_loading: "正在加载 Codex 用量…",
        refresh: "立即刷新",
        toggle_overlay: "显示标题栏用量",
        settings: "设置…",
        diagnose: "运行诊断",
        exit: "退出",
    },
    usage: UsageWord {
        remaining: "剩余",
        used: "已用",
    },
    settings: SettingsWindow {
        title: "CodexPlus 设置",
        page_title: "设置",
        page_subtitle: "管理用量显示、快捷指令和网络连接方式",
        display_card_title: "显示与刷新",
        language_label: "语言",
        language_zh_cn: "简体中文",
        language_english: "English",
        overlay_checkbox: "在 Codex 标题栏显示用量",
        refresh_label: "刷新间隔（秒）",
        usage_label: "额度显示",
        usage_remaining: "显示剩余额度",
        usage_used: "显示已用额度",
        reset_label: "重置时间显示",
        reset_countdown: "倒计时（如 5天14时）",
        reset_absolute: "到期时间（如 7月30日 19:34）",
        quick_prompts_card_title: "快捷指令",
        quick_prompts_help: "每行一条；最多 20 条，每条最多 2000 字。选择后会立即发送。",
        quick_prompts_invalid: "快捷指令最多 20 条，每条最多 2000 字。请修改后再保存。",
        proxy_card_title: "网络代理",
        proxy_env: "使用系统环境代理",
        proxy_direct: "强制直连（忽略所有代理）",
        proxy_custom: "使用自定义代理地址",
        proxy_address_label: "代理地址",
        proxy_help: "代理地址保存在本机配置文件中，打开设置时会再次显示。",
        cancel: "取消",
        save: "保存",
        invalid_proxy_message: "请输入有效的 http:// 或 https:// 自定义代理地址。",
        invalid_proxy_title: "CodexPlus 设置",
    },
    diagnose: DiagnoseDialog {
        title: "CodexPlus 诊断",
        status_ok: "诊断日志已生成",
        status_failed_prefix: "诊断日志写入失败：",
        log_label: "日志",
    },
    reason: ReasonLabel {
        auth: "认证失败",
        network: "网络异常",
        proxy: "代理异常",
        rate_limited: "请求受限",
        protocol: "服务协议变化",
        error: "未知错误",
    },
    months: [
        "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
    ],
};

const EN_TEXTS: UiTexts = UiTexts {
    language_name: "English",
    overlay: OverlayStatus {
        loading: "Codex usage · Loading…",
        unavailable: "Codex usage · Unavailable",
    },
    tooltip: TooltipStatus {
        loading: "Loading Codex usage…",
        updated_prefix: "Updated ",
        stale_prefix: "Stale (",
        stale_suffix: ") — updated ",
        error_prefix: "Error (",
        error_suffix: ") — updated ",
        bare_error_prefix: "Error: ",
    },
    window: WindowLabel {
        five_hour: "5h",
        weekly: "Weekly",
        short_window_prefix: "Short ",
        long_window_prefix: "Long ",
        other_window_prefix: "Other ",
    },
    countdown: Countdown { now: "now" },
    tray: TrayMenu {
        summary_loading: "Loading Codex usage…",
        refresh: "Refresh now",
        toggle_overlay: "Show title-bar usage",
        settings: "Settings…",
        diagnose: "Run diagnostics",
        exit: "Exit",
    },
    usage: UsageWord {
        remaining: "remaining",
        used: "used",
    },
    settings: SettingsWindow {
        title: "CodexPlus Settings",
        page_title: "Settings",
        page_subtitle: "Manage usage, quick prompts, and network access",
        display_card_title: "Display & refresh",
        language_label: "Language",
        language_zh_cn: "Simplified Chinese",
        language_english: "English",
        overlay_checkbox: "Show usage in the Codex title bar",
        refresh_label: "Refresh interval (seconds)",
        usage_label: "Quota display",
        usage_remaining: "Show remaining quota",
        usage_used: "Show used quota",
        reset_label: "Reset time display",
        reset_countdown: "Countdown (e.g. 5d14h)",
        reset_absolute: "Reset time (e.g. Jul 30 19:34)",
        quick_prompts_card_title: "Quick prompts",
        quick_prompts_help:
            "One per line; up to 20 prompts and 2,000 characters each. Selection sends immediately.",
        quick_prompts_invalid: "Use at most 20 quick prompts and 2,000 characters per prompt.",
        proxy_card_title: "Network proxy",
        proxy_env: "Use environment proxy",
        proxy_direct: "Connect directly (ignore all proxies)",
        proxy_custom: "Use a custom proxy URL",
        proxy_address_label: "Proxy URL",
        proxy_help: "The proxy URL is stored locally in the config file and shown again here.",
        cancel: "Cancel",
        save: "Save",
        invalid_proxy_message: "Please enter a valid http:// or https:// custom proxy URL.",
        invalid_proxy_title: "CodexPlus Settings",
    },
    diagnose: DiagnoseDialog {
        title: "CodexPlus diagnostics",
        status_ok: "Diagnostics log generated",
        status_failed_prefix: "Failed to write the diagnostics log: ",
        log_label: "Log",
    },
    reason: ReasonLabel {
        auth: "Authentication failed",
        network: "Network unavailable",
        proxy: "Proxy unavailable",
        rate_limited: "Rate limited",
        protocol: "Service protocol changed",
        error: "Unknown error",
    },
    months: [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
};

/// Return the static UI text bundle for the active language. Never panics.
pub const fn texts(language: Language) -> UiTexts {
    match language {
        Language::SimplifiedChinese => ZH_TEXTS,
        Language::English => EN_TEXTS,
    }
}

/// Localize the canonical reason keys produced by `AppState::reason_for`.
/// Unknown keys share the catch-all `error` slot.
pub fn localize_reason(language: Language, reason: &str) -> &'static str {
    let labels = texts(language).reason;
    match reason {
        "auth" => labels.auth,
        "network" => labels.network,
        "proxy" => labels.proxy,
        "rate limited" => labels.rate_limited,
        "protocol" => labels.protocol,
        _ => labels.error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_default_is_simplified_chinese() {
        assert!(matches!(Language::default(), Language::SimplifiedChinese));
    }

    #[test]
    fn text_lookup_covers_both_languages() {
        let zh = texts(Language::SimplifiedChinese);
        let en = texts(Language::English);
        assert_eq!(zh.window.five_hour, "5小时");
        assert_eq!(en.window.five_hour, "5h");
        assert_eq!(zh.countdown.now, "现在");
        assert_eq!(en.countdown.now, "now");
        assert_eq!(zh.settings.cancel, "取消");
        assert_eq!(en.settings.cancel, "Cancel");
    }

    #[test]
    fn localize_reason_maps_internal_keys() {
        assert_eq!(
            localize_reason(Language::English, "auth"),
            "Authentication failed"
        );
        assert_eq!(
            localize_reason(Language::SimplifiedChinese, "network"),
            "网络异常"
        );
        assert_eq!(
            localize_reason(Language::English, "unsupported"),
            "Unknown error"
        );
    }

    #[test]
    fn months_array_has_twelve_entries() {
        let zh = texts(Language::SimplifiedChinese);
        let en = texts(Language::English);
        assert_eq!(zh.months.len(), 12);
        assert_eq!(en.months.len(), 12);
        assert_eq!(en.months[6], "Jul");
    }
}
