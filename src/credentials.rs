//! Credential loading for the Codex CLI session.
//!
//! CodexPlus never *writes* Codex credentials. It only reads the access
//! token from the JSON file the CLI itself maintains so that the user does
//! not have to authenticate twice. The token is wrapped in
//! [`SecretString`] so that logs and `Debug` output cannot leak it by
//! accident.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::error::AppError;

/// Strongly-typed wrapper around a Codex OAuth access token.
#[derive(Clone)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[derive(Debug, Clone)]
pub struct CodexCredentials {
    pub access_token: SecretString,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct CredentialSignature {
    pub exists: bool,
    pub size: u64,
    pub modified_unix: Option<i64>,
}

/// Resolve the canonical `auth.json` location following Codex CLI conventions.
pub fn resolve_auth_path() -> Result<PathBuf, AppError> {
    if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
        let trimmed = trim_os_str(&codex_home);
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed).join("auth.json"));
        }
    }
    let home = dirs::home_dir().ok_or(AppError::AuthPathUnset)?;
    Ok(home.join(".codex").join("auth.json"))
}

fn trim_os_str(value: &OsString) -> String {
    value.to_string_lossy().trim().to_string()
}

pub fn credential_signature(path: &Path) -> CredentialSignature {
    match fs::metadata(path) {
        Ok(m) => CredentialSignature {
            exists: true,
            size: m.len(),
            modified_unix: m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64),
        },
        Err(_) => CredentialSignature {
            exists: false,
            size: 0,
            modified_unix: None,
        },
    }
}

pub fn read_credentials(path: &Path) -> Result<CodexCredentials, AppError> {
    let raw = fs::read_to_string(path).map_err(|err| match err.kind() {
        io::ErrorKind::NotFound => AppError::AuthFileMissing,
        _ => AppError::AuthFileUnreadable,
    })?;
    parse_credentials(&raw)
}

fn parse_credentials(raw: &str) -> Result<CodexCredentials, AppError> {
    #[derive(serde::Deserialize)]
    struct AuthFile {
        tokens: Option<AuthTokens>,
    }
    #[derive(serde::Deserialize)]
    struct AuthTokens {
        access_token: Option<String>,
        account_id: Option<String>,
    }
    let parsed: AuthFile = serde_json::from_str(raw).map_err(|_| AppError::AuthFileInvalid)?;
    let tokens = parsed.tokens.ok_or(AppError::AccessTokenMissing)?;
    let token = tokens
        .access_token
        .and_then(non_empty)
        .ok_or(AppError::AccessTokenMissing)?;
    let account_id = tokens.account_id.and_then(non_empty);
    Ok(CodexCredentials {
        access_token: SecretString::new(token),
        account_id,
    })
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_tokens_with_optional_account() {
        let raw = r#"{"tokens":{"access_token":"abc","account_id":"acc-1"}}"#;
        let c = parse_credentials(raw).expect("ok");
        assert_eq!(c.access_token.expose(), "abc");
        assert_eq!(c.account_id.as_deref(), Some("acc-1"));
    }

    #[test]
    fn rejects_missing_tokens() {
        let raw = r#"{"OPENAI_API_KEY":"xyz"}"#;
        let err = parse_credentials(raw).expect_err("must reject");
        assert_eq!(err, AppError::AccessTokenMissing);
    }

    #[test]
    fn rejects_empty_access_token() {
        let raw = r#"{"tokens":{"access_token":""}}"#;
        assert_eq!(
            parse_credentials(raw).expect_err("must reject"),
            AppError::AccessTokenMissing
        );
    }

    #[test]
    fn rejects_invalid_json() {
        let raw = "not json";
        assert_eq!(
            parse_credentials(raw).expect_err("must reject"),
            AppError::AuthFileInvalid
        );
    }

    #[test]
    fn secret_string_debug_is_redacted() {
        let secret = SecretString::new("super-secret".to_string());
        let formatted = format!("{secret:?}");
        assert!(!formatted.contains("super-secret"));
    }

    #[test]
    fn resolve_auth_path_uses_home_when_no_env() {
        // SAFETY: this test only inspects `CODEX_HOME`; no concurrent mutation.
        unsafe { std::env::remove_var("CODEX_HOME") };
        let path = resolve_auth_path().expect("ok");
        assert!(path.ends_with(".codex/auth.json"));
    }

    #[test]
    fn resolve_auth_path_respects_codex_home() {
        // SAFETY: racy test isolation; runs sequentially under cargo test.
        unsafe { std::env::set_var("CODEX_HOME", "C:/Users/me/MyCodexHome") };
        let path = resolve_auth_path().expect("ok");
        unsafe { std::env::remove_var("CODEX_HOME") };
        assert!(path.ends_with("MyCodexHome/auth.json"));
    }

    #[test]
    fn credential_signature_reports_missing_file() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("auth.json");
        let sig = credential_signature(&path);
        assert!(!sig.exists);
        assert_eq!(sig.size, 0);
    }

    #[test]
    fn credential_signature_reports_existing_file() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("auth.json");
        let payload = br#"{"tokens":{"access_token":"abc"}}"#;
        let mut file = std::fs::File::create(&path).expect("create");
        file.write_all(payload).expect("write");
        let sig = credential_signature(&path);
        assert!(sig.exists);
        assert_eq!(sig.size as usize, payload.len());
    }
}
