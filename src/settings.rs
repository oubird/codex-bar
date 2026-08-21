//! Persistent non-secret application settings.
//!
//! Usage snapshots stay in `usage-cache.json`; settings use a separate file.
//! Custom proxy addresses are stored here as plain text because the desktop
//! configuration is intentionally user-visible through the settings window.
//! Codex `auth.json` access tokens remain the only thing the app never
//! persists, logs, or surfaces in errors.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    #[default]
    Environment,
    Direct,
    Custom,
}

/// Whether the title-bar overlay shows the remaining quota (`剩余`) or the
/// consumed quota (`已用`). Treated as ordinary UI configuration. Tone colour
/// thresholds remain driven by the server-reported `used_percent`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageDisplay {
    #[default]
    Remaining,
    Used,
}

/// How reset time is rendered in the title-bar overlay and tray tooltip.
/// `Countdown` shows a relative `5天14时` style value; `Absolute` shows the
/// local clock time of the next reset, formatted as `M月D日 HH:MM`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetTimeDisplay {
    #[default]
    Countdown,
    Absolute,
}

/// Active UI language for the title-bar overlay, tray tooltip, tray menu, and
/// settings window. Persisted together with the other display options so the
/// same selection survives restart. Defaults to `SimplifiedChinese`; legacy
/// `settings.json` files written before this field existed deserialize
/// through `#[serde(default)]` so the previous setup is preserved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    SimplifiedChinese,
    English,
}

pub const MAX_QUICK_PROMPTS: usize = 20;
pub const MAX_QUICK_PROMPT_CHARS: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickPromptError {
    TooMany,
    TooLong,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_true")]
    pub overlay_enabled: bool,
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval_secs: u32,
    #[serde(default)]
    pub proxy_mode: ProxyMode,
    /// Persisted user-supplied proxy URL for `ProxyMode::Custom`. Treated as
    /// ordinary configuration; never written to logs, errors, or diagnostics.
    #[serde(default)]
    pub custom_proxy_url: Option<String>,
    /// Legacy field kept for forward compatibility with earlier releases that
    /// stored only a credential target name. Older `settings.json` files
    /// still deserialize, but the value is cleared after the URL persistence
    /// change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_proxy_credential_target: Option<String>,
    #[serde(default)]
    pub usage_display: UsageDisplay,
    #[serde(default)]
    pub reset_time_display: ResetTimeDisplay,
    #[serde(default)]
    pub language: Language,
    /// User-authored high-frequency prompts. These are ordinary local UI
    /// settings and are never included in logs or diagnostics.
    #[serde(default)]
    pub quick_prompts: Vec<String>,
}

const SCHEMA_VERSION: u32 = 2;

const fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}
const fn default_true() -> bool {
    true
}
const fn default_refresh_interval() -> u32 {
    300
}

impl Settings {
    pub fn proxy_policy(&self) -> crate::provider::ProxyPolicy {
        match self.proxy_mode {
            ProxyMode::Environment => crate::provider::ProxyPolicy::Environment,
            ProxyMode::Direct => crate::provider::ProxyPolicy::Direct,
            ProxyMode::Custom => crate::provider::ProxyPolicy::CustomCredential,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            overlay_enabled: true,
            refresh_interval_secs: default_refresh_interval(),
            proxy_mode: ProxyMode::Environment,
            custom_proxy_url: None,
            custom_proxy_credential_target: None,
            usage_display: UsageDisplay::default(),
            reset_time_display: ResetTimeDisplay::default(),
            language: Language::default(),
            quick_prompts: Vec::new(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct SettingsPatch {
    pub overlay_enabled: Option<bool>,
    pub refresh_interval_secs: Option<u32>,
    pub proxy_mode: Option<ProxyMode>,
    /// Custom proxy URL the user entered in the settings window. Trimmed of
    /// surrounding whitespace before storage. An empty string clears the
    /// stored URL regardless of mode.
    pub custom_proxy_url: Option<String>,
    pub usage_display: Option<UsageDisplay>,
    pub reset_time_display: Option<ResetTimeDisplay>,
    pub language: Option<Language>,
    pub quick_prompts: Option<Vec<String>>,
}

impl SettingsPatch {
    pub fn apply_to(self, settings: &mut Settings) {
        if let Some(value) = self.overlay_enabled {
            settings.overlay_enabled = value;
        }
        if let Some(value) = self.refresh_interval_secs {
            settings.refresh_interval_secs = value.clamp(60, 3600);
        }
        if let Some(mode) = self.proxy_mode {
            settings.proxy_mode = mode;
        }
        if let Some(value) = self.custom_proxy_url {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                settings.custom_proxy_url = None;
            } else {
                settings.custom_proxy_url = Some(trimmed.to_owned());
            }
        }
        if settings.proxy_mode != ProxyMode::Custom {
            settings.custom_proxy_url = None;
        }
        if let Some(value) = self.usage_display {
            settings.usage_display = value;
        }
        if let Some(value) = self.reset_time_display {
            settings.reset_time_display = value;
        }
        if let Some(value) = self.language {
            settings.language = value;
        }
        if let Some(value) = self.quick_prompts {
            settings.quick_prompts = normalize_quick_prompts(value);
        }
        settings.custom_proxy_credential_target = None;
        settings.schema_version = SCHEMA_VERSION;
    }
}

pub fn settings_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join("CodexPlus").join("settings.json"))
}

pub fn load() -> Settings {
    let Some(path) = settings_path() else {
        return Settings::default();
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return Settings::default();
    };
    let Ok(settings) = serde_json::from_str::<Settings>(&raw) else {
        return Settings::default();
    };
    normalize(settings)
}

fn normalize(mut settings: Settings) -> Settings {
    if settings.schema_version != SCHEMA_VERSION {
        return Settings::default();
    }
    settings.refresh_interval_secs = settings.refresh_interval_secs.clamp(60, 3600);
    if settings.proxy_mode != ProxyMode::Custom {
        settings.custom_proxy_url = None;
    } else if let Some(url) = settings.custom_proxy_url.as_deref() {
        if !validate_custom_proxy_url(url) {
            settings.custom_proxy_url = None;
        }
    }
    settings.custom_proxy_credential_target = None;
    settings.quick_prompts = normalize_quick_prompts(settings.quick_prompts);
    settings
}

pub fn parse_quick_prompts(value: &str) -> Result<Vec<String>, QuickPromptError> {
    let prompts = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if prompts.len() > MAX_QUICK_PROMPTS {
        return Err(QuickPromptError::TooMany);
    }
    if prompts
        .iter()
        .any(|prompt| prompt.chars().count() > MAX_QUICK_PROMPT_CHARS)
    {
        return Err(QuickPromptError::TooLong);
    }
    Ok(prompts.into_iter().map(str::to_owned).collect())
}

fn normalize_quick_prompts(prompts: Vec<String>) -> Vec<String> {
    prompts
        .into_iter()
        .map(|prompt| prompt.trim().to_owned())
        .filter(|prompt| !prompt.is_empty())
        .filter(|prompt| !prompt.contains(['\r', '\n']))
        .filter(|prompt| prompt.chars().count() <= MAX_QUICK_PROMPT_CHARS)
        .take(MAX_QUICK_PROMPTS)
        .collect()
}

pub fn save(settings: &Settings) -> io::Result<()> {
    let path = settings_path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "local app data unavailable"))?;
    let parent = path.parent().expect("settings path parent");
    fs::create_dir_all(parent)?;
    let data = serde_json::to_vec_pretty(settings)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let temporary = path.with_extension("json.tmp");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(&data)?;
    file.sync_all()?;
    fs::rename(temporary, path)
}

/// Returns `true` if `value` is a syntactically valid custom proxy URL using
/// the `http` or `https` scheme and containing a host.
pub fn validate_custom_proxy_url(value: &str) -> bool {
    match url::Url::parse(value.trim()) {
        Ok(parsed) => matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_use_environment_proxy() {
        let settings = Settings::default();
        assert!(settings.overlay_enabled);
        assert_eq!(settings.proxy_mode, ProxyMode::Environment);
        assert_eq!(settings.refresh_interval_secs, 300);
        assert_eq!(settings.custom_proxy_url, None);
        assert_eq!(settings.usage_display, UsageDisplay::Remaining);
        assert_eq!(settings.reset_time_display, ResetTimeDisplay::Countdown);
        assert_eq!(settings.language, Language::SimplifiedChinese);
        assert!(settings.quick_prompts.is_empty());
    }

    #[test]
    fn patch_applies_usage_and_reset_displays() {
        let mut settings = Settings::default();
        SettingsPatch {
            usage_display: Some(UsageDisplay::Used),
            reset_time_display: Some(ResetTimeDisplay::Absolute),
            ..Default::default()
        }
        .apply_to(&mut settings);
        assert_eq!(settings.usage_display, UsageDisplay::Used);
        assert_eq!(settings.reset_time_display, ResetTimeDisplay::Absolute);
    }

    #[test]
    fn patch_applies_language() {
        let mut settings = Settings::default();
        SettingsPatch {
            language: Some(Language::English),
            ..Default::default()
        }
        .apply_to(&mut settings);
        assert_eq!(settings.language, Language::English);
    }

    #[test]
    fn quick_prompts_parse_one_non_empty_instruction_per_line() {
        assert_eq!(
            parse_quick_prompts("  审查当前改动  \r\n\r\n给出实施计划\n").unwrap(),
            vec!["审查当前改动", "给出实施计划"]
        );
    }

    #[test]
    fn quick_prompts_enforce_count_and_length_limits() {
        let too_many = (0..=MAX_QUICK_PROMPTS)
            .map(|index| format!("指令{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            parse_quick_prompts(&too_many),
            Err(QuickPromptError::TooMany)
        );
        let too_long = "好".repeat(MAX_QUICK_PROMPT_CHARS + 1);
        assert_eq!(
            parse_quick_prompts(&too_long),
            Err(QuickPromptError::TooLong)
        );
    }

    #[test]
    fn language_round_trips_via_serde() {
        let mut settings = Settings::default();
        SettingsPatch {
            language: Some(Language::English),
            ..Default::default()
        }
        .apply_to(&mut settings);
        let serialized = serde_json::to_string(&settings).expect("serialize");
        assert!(serialized.contains("\"language\":\"english\""));
        let parsed: Settings = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(parsed.language, Language::English);
    }

    #[test]
    fn legacy_v2_settings_json_adopts_language_default() {
        // The historical settings.json files written before this change had no
        // language field. Deserializing that shape should still load and fall
        // back to SimplifiedChinese without losing the other persisted values.
        let raw = r#"{"schema_version":2,"overlay_enabled":false,"refresh_interval_secs":120,"proxy_mode":"direct","custom_proxy_url":null}"#;
        let parsed: Settings = serde_json::from_str(raw).expect("deserialize");
        assert_eq!(parsed.language, Language::SimplifiedChinese);
        assert!(!parsed.overlay_enabled);
        assert_eq!(parsed.refresh_interval_secs, 120);
        assert_eq!(parsed.proxy_mode, ProxyMode::Direct);
    }

    #[test]
    fn usage_and_reset_displays_round_trip_via_serde() {
        let mut settings = Settings::default();
        SettingsPatch {
            usage_display: Some(UsageDisplay::Used),
            reset_time_display: Some(ResetTimeDisplay::Absolute),
            ..Default::default()
        }
        .apply_to(&mut settings);
        let serialized = serde_json::to_string(&settings).expect("serialize");
        assert!(serialized.contains("\"usage_display\":\"used\""));
        assert!(serialized.contains("\"reset_time_display\":\"absolute\""));
        let parsed: Settings = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(parsed.usage_display, UsageDisplay::Used);
        assert_eq!(parsed.reset_time_display, ResetTimeDisplay::Absolute);
    }

    #[test]
    fn legacy_v2_settings_json_adopts_defaults_for_new_fields() {
        // The historical settings.json files written before this change had no
        // usage_display / reset_time_display fields. Deserializing that shape
        // should still load and fall back to the new defaults.
        let raw = r#"{"schema_version":2,"overlay_enabled":true,"refresh_interval_secs":300,"proxy_mode":"environment","custom_proxy_url":null}"#;
        let parsed: Settings = serde_json::from_str(raw).expect("deserialize");
        assert_eq!(parsed.usage_display, UsageDisplay::Remaining);
        assert_eq!(parsed.reset_time_display, ResetTimeDisplay::Countdown);
        assert_eq!(parsed.language, Language::SimplifiedChinese);
    }

    #[test]
    fn patch_clamps_refresh_interval() {
        let mut settings = Settings::default();
        SettingsPatch {
            refresh_interval_secs: Some(1),
            ..Default::default()
        }
        .apply_to(&mut settings);
        assert_eq!(settings.refresh_interval_secs, 60);
        SettingsPatch {
            refresh_interval_secs: Some(99_999),
            ..Default::default()
        }
        .apply_to(&mut settings);
        assert_eq!(settings.refresh_interval_secs, 3600);
    }

    #[test]
    fn custom_proxy_url_round_trips_when_valid() {
        let mut settings = Settings::default();
        SettingsPatch {
            proxy_mode: Some(ProxyMode::Custom),
            custom_proxy_url: Some("  http://127.0.0.1:7890  ".to_string()),
            ..Default::default()
        }
        .apply_to(&mut settings);
        assert_eq!(
            settings.custom_proxy_url.as_deref(),
            Some("http://127.0.0.1:7890")
        );
        let serialized = serde_json::to_string(&settings).expect("serialize");
        assert!(serialized.contains("http://127.0.0.1:7890"));
    }

    #[test]
    fn switching_away_from_custom_clears_url() {
        let mut settings = Settings::default();
        SettingsPatch {
            proxy_mode: Some(ProxyMode::Custom),
            custom_proxy_url: Some("http://127.0.0.1:7890".to_string()),
            ..Default::default()
        }
        .apply_to(&mut settings);
        assert!(settings.custom_proxy_url.is_some());

        SettingsPatch {
            proxy_mode: Some(ProxyMode::Direct),
            ..Default::default()
        }
        .apply_to(&mut settings);
        assert_eq!(settings.custom_proxy_url, None);
    }

    #[test]
    fn empty_custom_proxy_url_clears_existing_value() {
        let mut settings = Settings::default();
        SettingsPatch {
            proxy_mode: Some(ProxyMode::Custom),
            custom_proxy_url: Some("http://127.0.0.1:7890".to_string()),
            ..Default::default()
        }
        .apply_to(&mut settings);

        SettingsPatch {
            proxy_mode: Some(ProxyMode::Custom),
            custom_proxy_url: Some(String::new()),
            ..Default::default()
        }
        .apply_to(&mut settings);
        assert_eq!(settings.custom_proxy_url, None);
    }

    #[test]
    fn invalid_custom_proxy_url_is_dropped_on_load() {
        let raw = r#"{"schema_version":2,"overlay_enabled":true,"refresh_interval_secs":300,"proxy_mode":"custom","custom_proxy_url":"not a url"}"#;
        let settings: Settings = serde_json::from_str(raw).expect("deserialize");
        let normalized = normalize(settings);
        assert_eq!(normalized.custom_proxy_url, None);
    }

    #[test]
    fn validate_custom_proxy_url_accepts_http_and_https() {
        assert!(validate_custom_proxy_url("http://127.0.0.1:7890"));
        assert!(validate_custom_proxy_url("https://example.test:443"));
        assert!(!validate_custom_proxy_url("socks5://127.0.0.1:1080"));
        assert!(!validate_custom_proxy_url(""));
        assert!(!validate_custom_proxy_url("not a url"));
    }

    #[test]
    fn incompatible_schema_reverts_to_defaults() {
        let settings = normalize(Settings {
            schema_version: SCHEMA_VERSION + 1,
            overlay_enabled: false,
            refresh_interval_secs: 60,
            proxy_mode: ProxyMode::Custom,
            custom_proxy_url: Some("http://127.0.0.1:7890".to_string()),
            custom_proxy_credential_target: Some("unexpected".to_owned()),
            usage_display: UsageDisplay::Used,
            reset_time_display: ResetTimeDisplay::Absolute,
            language: Language::English,
            quick_prompts: vec!["should be reset".to_owned()],
        });
        assert!(settings.overlay_enabled);
        assert_eq!(settings.proxy_mode, ProxyMode::Environment);
        assert_eq!(settings.refresh_interval_secs, 300);
        assert_eq!(settings.custom_proxy_url, None);
        assert_eq!(settings.language, Language::SimplifiedChinese);
    }
}
