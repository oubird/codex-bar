//! Error types for CodexPlus.
//!
//! Errors never carry raw request headers, response bodies, tokens or paths
//! beyond what's necessary for diagnostics. [`AppError::public_message`]
//! returns a fixed string suitable for UI/tooltip/log.

use crate::model::WindowKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppError {
    // Credential / auth path issues.
    AuthPathUnset,
    AuthFileMissing,
    AuthFileUnreadable,
    AuthFileInvalid,
    AccessTokenMissing,

    // Wire problems.
    Unauthorized,
    Forbidden,
    RateLimited { retry_after_secs: Option<u64> },
    /// Connection/transport failure. Deliberately stores no library detail:
    /// proxy transport errors can embed a proxy URL or credentials.
    Network,
    /// A configured proxy URL could not be parsed safely.
    ProxyConfigurationInvalid,
    Timeout,
    HttpStatus(u16),
    RedirectRejected,
    ResponseTooLarge,

    // Protocol problems (must never be silently substituted).
    ProtocolChange(&'static str),
    WindowMissing(WindowKind),
    WindowDurationMismatch {
        kind: WindowKind,
        actual_seconds: i64,
    },
    PercentInvalid(WindowKind),
    ResetInvalid(WindowKind),

    // Persistence.
    CacheInvalid,
    CacheIo(String),

    // Catch-all bucket, also used for IPC plumbing errors.
    Internal(String),
}

impl AppError {
    /// How a refresh loop should react to this error.
    pub fn retry_class(&self) -> RetryClass {
        match self {
            AppError::Unauthorized
            | AppError::Forbidden
            | AppError::AuthFileMissing
            | AppError::AuthFileUnreadable
            | AppError::AuthFileInvalid
            | AppError::AccessTokenMissing
            | AppError::AuthPathUnset => RetryClass::AuthFailure,

            AppError::RateLimited { .. } => RetryClass::RateLimited,
            AppError::Timeout | AppError::Network | AppError::ProxyConfigurationInvalid => {
                RetryClass::Network
            },
            AppError::HttpStatus(code) if *code >= 500 => RetryClass::Network,
            AppError::HttpStatus(_) => RetryClass::NoRetry,

            AppError::RedirectRejected
            | AppError::ResponseTooLarge
            | AppError::ProtocolChange(_)
            | AppError::WindowMissing(_)
            | AppError::WindowDurationMismatch { .. }
            | AppError::PercentInvalid(_)
            | AppError::ResetInvalid(_) => RetryClass::Protocol,

            AppError::CacheInvalid | AppError::CacheIo(_) => RetryClass::Cache,
            AppError::Internal(_) => RetryClass::Network,
        }
    }

    /// Stable, safe text that is safe to display.
    pub fn public_message(&self) -> &'static str {
        match self {
            AppError::AuthPathUnset => "Codex configuration directory is not available.",
            AppError::AuthFileMissing => "Codex is not logged in. Run `codex login` first.",
            AppError::AuthFileUnreadable => "Codex credentials could not be read.",
            AppError::AuthFileInvalid => "Codex credentials are malformed.",
            AppError::AccessTokenMissing => "Codex credentials are missing an access token.",
            AppError::Unauthorized | AppError::Forbidden => {
                "Codex session expired. Run `codex login` to refresh."
            }
            AppError::RateLimited { .. } => "Codex rate limit reached. Will retry shortly.",
            AppError::Network => "Network is unavailable.",
            AppError::ProxyConfigurationInvalid => "Configured HTTP proxy is invalid.",
            AppError::Timeout => "Request timed out.",
            AppError::HttpStatus(_) => "Codex service responded with an error.",
            AppError::RedirectRejected => "Unexpected redirect from Codex service.",
            AppError::ResponseTooLarge => "Codex service response was unexpectedly large.",
            AppError::ProtocolChange(_) => "Codex usage schema changed. Waiting for an update.",
            AppError::WindowMissing(_) => "Codex usage data is missing a required window.",
            AppError::WindowDurationMismatch { .. } => "Unexpected usage window duration.",
            AppError::PercentInvalid(_) => "Invalid usage percentage.",
            AppError::ResetInvalid(_) => "Invalid reset timestamp.",
            AppError::CacheInvalid => "Cached usage data is unusable.",
            AppError::CacheIo(_) => "Could not write the local cache.",
            AppError::Internal(_) => "Internal error.",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryClass {
    Network,
    RateLimited,
    AuthFailure,
    Protocol,
    Cache,
    NoRetry,
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Do not leak details through Display; Debug has more but stays local.
        f.write_str(self.public_message())
    }
}

impl std::error::Error for AppError {}
