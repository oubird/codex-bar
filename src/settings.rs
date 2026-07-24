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
    settings
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
        });
        assert!(settings.overlay_enabled);
        assert_eq!(settings.proxy_mode, ProxyMode::Environment);
        assert_eq!(settings.refresh_interval_secs, 300);
        assert_eq!(settings.custom_proxy_url, None);
    }
}
