//! Provider layer for CodexPlus.
//!
//! `CodexProvider` is the only place that knows about the upstream
//! `wham/usage` endpoint, headers, and JSON shape. The rest of the
//! application only depends on the domain model in
//! [`crate::model`].

mod codex;

pub use codex::{
    codex_default_endpoint, CodexProvider, ProviderTransport, ProxyConfiguration, ProxyPolicy,
    ProxySource, UreqTransport, UsageResponse,
};

use crate::credentials::CodexCredentials;
use crate::error::AppError;
use crate::model::UsageSnapshot;

pub trait UsageProvider: Send + Sync {
    fn fetch(&self, credentials: &CodexCredentials) -> Result<UsageSnapshot, AppError>;
}
