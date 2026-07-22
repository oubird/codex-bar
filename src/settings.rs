//! Persistent non-secret application settings.
//!
//! Usage snapshots stay in `usage-cache.json`; settings use a separate file.
//! Custom proxy credentials are intentionally not represented here. The UI
//! stores them through Windows Credential Manager and this file keeps only a
//! stable credential target name.

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
    #[serde(default)]
    pub custom_proxy_credential_target: Option<String>,
}

const SCHEMA_VERSION: u32 = 1;
pub const CUSTOM_PROXY_CREDENTIAL_TARGET: &str = "CodexPlus.ProxyUrl.v1";

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
            custom_proxy_credential_target: None,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct SettingsPatch {
    pub overlay_enabled: Option<bool>,
    pub refresh_interval_secs: Option<u32>,
    pub proxy_mode: Option<ProxyMode>,
    pub custom_proxy_configured: Option<bool>,
}

impl SettingsPatch {
    pub fn apply_to(self, settings: &mut Settings) {
        if let Some(value) = self.overlay_enabled {
            settings.overlay_enabled = value;
        }
        if let Some(value) = self.refresh_interval_secs {
            settings.refresh_interval_secs = value.clamp(60, 3600);
        }
        if let Some(value) = self.proxy_mode {
            settings.proxy_mode = value;
        }
        if let Some(configured) = self.custom_proxy_configured {
            settings.custom_proxy_credential_target =
                configured.then(|| CUSTOM_PROXY_CREDENTIAL_TARGET.to_owned());
        }
        if settings.proxy_mode != ProxyMode::Custom {
            settings.custom_proxy_credential_target = None;
        }
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
        settings.custom_proxy_credential_target = None;
    }
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

#[cfg(windows)]
pub fn save_custom_proxy_url(proxy_url: &str) -> Result<(), String> {
    use windows::core::PWSTR;
    use windows::Win32::Security::Credentials::{
        CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    };

    let parsed = url::Url::parse(proxy_url).map_err(|_| "Invalid proxy URL.".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("Invalid proxy URL.".to_string());
    }

    let mut target: Vec<u16> = CUSTOM_PROXY_CREDENTIAL_TARGET
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut username: Vec<u16> = "CodexPlus".encode_utf16().chain(Some(0)).collect();
    let mut secret = proxy_url.as_bytes().to_vec();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_mut_ptr()),
        CredentialBlobSize: secret.len() as u32,
        CredentialBlob: secret.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: PWSTR(username.as_mut_ptr()),
        ..Default::default()
    };
    unsafe { CredWriteW(&credential, 0) }
        .map_err(|_| "Could not store custom proxy securely.".to_string())
}

#[cfg(windows)]
pub fn load_custom_proxy_url() -> Result<Option<String>, String> {
    use windows::core::PCWSTR;
    use windows::Win32::Security::Credentials::{
        CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
    };

    let target: Vec<u16> = CUSTOM_PROXY_CREDENTIAL_TARGET
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
    let result = unsafe {
        CredReadW(
            PCWSTR(target.as_ptr()),
            CRED_TYPE_GENERIC,
            0,
            &mut credential,
        )
    };
    if result.is_err() {
        return Ok(None);
    }
    let bytes = unsafe {
        std::slice::from_raw_parts(
            (*credential).CredentialBlob,
            (*credential).CredentialBlobSize as usize,
        )
    };
    let value = std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| "Stored custom proxy is invalid.".to_string());
    unsafe { CredFree(credential.cast()) };
    value.map(Some)
}

#[cfg(windows)]
pub fn delete_custom_proxy_url() -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};

    let target: Vec<u16> = CUSTOM_PROXY_CREDENTIAL_TARGET
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, 0) }
        .map_err(|_| "Could not remove custom proxy securely.".to_string())
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
    fn switching_away_from_custom_clears_credential_metadata() {
        let mut settings = Settings::default();
        SettingsPatch {
            proxy_mode: Some(ProxyMode::Custom),
            custom_proxy_configured: Some(true),
            ..Default::default()
        }
        .apply_to(&mut settings);
        assert_eq!(
            settings.custom_proxy_credential_target.as_deref(),
            Some(CUSTOM_PROXY_CREDENTIAL_TARGET)
        );

        SettingsPatch {
            proxy_mode: Some(ProxyMode::Direct),
            ..Default::default()
        }
        .apply_to(&mut settings);
        assert_eq!(settings.custom_proxy_credential_target, None);
    }

    #[test]
    fn incompatible_schema_reverts_to_defaults() {
        let settings = normalize(Settings {
            schema_version: SCHEMA_VERSION + 1,
            overlay_enabled: false,
            refresh_interval_secs: 60,
            proxy_mode: ProxyMode::Direct,
            custom_proxy_credential_target: Some("unexpected".to_owned()),
        });
        assert!(settings.overlay_enabled);
        assert_eq!(settings.proxy_mode, ProxyMode::Environment);
        assert_eq!(settings.refresh_interval_secs, 300);
    }

    #[test]
    fn custom_proxy_setting_never_contains_url_field() {
        let mut settings = Settings::default();
        SettingsPatch {
            proxy_mode: Some(ProxyMode::Custom),
            custom_proxy_configured: Some(true),
            ..Default::default()
        }
        .apply_to(&mut settings);
        let serialized = serde_json::to_string(&settings).expect("serialize");
        assert!(serialized.contains(CUSTOM_PROXY_CREDENTIAL_TARGET));
        assert!(!serialized.contains("127.0.0.1"));
        assert!(!serialized.contains("password"));
    }
}
