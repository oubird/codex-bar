//! Direct client for the Codex `/wham/usage` endpoint.
//!
//! The endpoint is **internal** and not a documented public API; we
//! isolate all knowledge of it inside this module. When the schema
//! changes, the rest of the application is unaffected as long as this
//! module still produces a [`UsageSnapshot`].

use std::time::Duration;

use crate::credentials::CodexCredentials;
use crate::error::AppError;
use crate::model::{
    RateWindow, UsageSnapshot, WindowKind, CLOCK_SKEW_SECONDS, FIVE_HOUR_SECONDS,
    MAX_WINDOW_SECONDS, MIN_WINDOW_SECONDS,
};
use crate::provider::UsageProvider;

/// The fixed endpoint this MVP is allowed to talk to.
///
/// We intentionally do NOT read `config.toml`'s `chatgpt_base_url`
/// override. The original reasoning is that customising the base URL
/// silently redirects the bearer token to a third-party host.
pub const fn codex_default_endpoint() -> &'static str {
    "https://chatgpt.com/backend-api/wham/usage"
}

const USER_AGENT: &str = "codexplus/0.1";
const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(20);

/// Where the active proxy policy came from. This safe label is the only
/// proxy detail callers may show in diagnostics; the URL stays private.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxySource {
    Direct,
    CodexplusHttpsProxy,
    HttpsProxy,
    HttpProxy,
}

impl ProxySource {
    pub fn diagnostic_label(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::CodexplusHttpsProxy => "CODEXPLUS_HTTPS_PROXY",
            Self::HttpsProxy => "HTTPS_PROXY",
            Self::HttpProxy => "HTTP_PROXY",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyPolicy {
    Environment,
    Direct,
    CustomCredential,
}

/// Validated proxy state. The raw proxy URL and `ureq::Proxy` are never
/// exposed, formatted, cached, or persisted because either can include
/// proxy credentials.
pub struct ProxyConfiguration {
    source: ProxySource,
    proxy: Option<ureq::Proxy>,
}

impl ProxyConfiguration {
    pub fn from_environment() -> Result<Self, AppError> {
        Self::from_env_lookup(|key| std::env::var_os(key))
    }

    pub fn source(&self) -> ProxySource {
        self.source
    }

    pub fn from_policy(policy: ProxyPolicy) -> Result<Self, AppError> {
        match policy {
            ProxyPolicy::Environment => Self::from_environment(),
            ProxyPolicy::Direct => Ok(Self {
                source: ProxySource::Direct,
                proxy: None,
            }),
            ProxyPolicy::CustomCredential => {
                let url = crate::settings::load_custom_proxy_url()
                    .map_err(|_| AppError::CustomProxyUnavailable)?
                    .ok_or(AppError::CustomProxyUnavailable)?;
                Self::from_custom_url(&url)
            }
        }
    }

    fn from_custom_url(value: &str) -> Result<Self, AppError> {
        let parsed = url::Url::parse(value).map_err(|_| AppError::ProxyConfigurationInvalid)?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return Err(AppError::ProxyConfigurationInvalid);
        }
        let proxy = ureq::Proxy::new(value).map_err(|_| AppError::ProxyConfigurationInvalid)?;
        Ok(Self {
            source: ProxySource::HttpProxy,
            proxy: Some(proxy),
        })
    }

    fn from_env_lookup(
        lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
    ) -> Result<Self, AppError> {
        const PROXY_ENVIRONMENT: [(&str, ProxySource); 5] = [
            ("CODEXPLUS_HTTPS_PROXY", ProxySource::CodexplusHttpsProxy),
            ("HTTPS_PROXY", ProxySource::HttpsProxy),
            ("https_proxy", ProxySource::HttpsProxy),
            ("HTTP_PROXY", ProxySource::HttpProxy),
            ("http_proxy", ProxySource::HttpProxy),
        ];

        for (key, source) in PROXY_ENVIRONMENT {
            let Some(raw) = lookup(key) else {
                continue;
            };
            let Some(value) = raw.to_str() else {
                return Err(AppError::ProxyConfigurationInvalid);
            };
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            let parsed = url::Url::parse(value).map_err(|_| AppError::ProxyConfigurationInvalid)?;
            if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
                return Err(AppError::ProxyConfigurationInvalid);
            }
            let proxy = ureq::Proxy::new(value).map_err(|_| AppError::ProxyConfigurationInvalid)?;
            return Ok(Self {
                source,
                proxy: Some(proxy),
            });
        }
        Ok(Self {
            source: ProxySource::Direct,
            proxy: None,
        })
    }
}

#[derive(Clone)]
pub struct UsageResponse {
    body: String,
}

impl UsageResponse {
    pub fn from_raw(body: String) -> Self {
        Self { body }
    }

    pub fn body(&self) -> &str {
        &self.body
    }
}

/// Transport abstraction so we can stub out the network in tests without
/// touching the production endpoint.
pub trait ProviderTransport: Send + Sync {
    fn send(&self, request: ProviderRequest) -> Result<UsageResponse, AppError>;
}

#[derive(Debug, Clone)]
pub struct ProviderRequest {
    pub endpoint: String,
    pub bearer_token: String,
    pub account_id: Option<String>,
}

pub struct CodexProvider {
    endpoint: &'static str,
    transport: Box<dyn ProviderTransport>,
}

impl CodexProvider {
    pub fn with_transport(endpoint: &'static str, transport: Box<dyn ProviderTransport>) -> Self {
        Self {
            endpoint,
            transport,
        }
    }

    pub fn with_default(transport: Box<dyn ProviderTransport>) -> Self {
        Self::with_transport(codex_default_endpoint(), transport)
    }

    fn parse(&self, response: &UsageResponse) -> Result<UsageSnapshot, AppError> {
        let json: serde_json::Value =
            serde_json::from_str(response.body()).map_err(|_| AppError::ProtocolChange("json"))?;

        let rate_limit = json
            .get("rate_limit")
            .ok_or(AppError::ProtocolChange("missing rate_limit"))?;
        let primary = rate_limit
            .get("primary_window")
            .ok_or(AppError::WindowMissing(WindowKind::FiveHour))?;
        let secondary = rate_limit
            .get("secondary_window")
            .ok_or(AppError::WindowMissing(WindowKind::Weekly))?;

        let now = UsageSnapshot::now();
        let five = self.parse_window(WindowKind::FiveHour, primary, now)?;
        let week = self.parse_window(WindowKind::Weekly, secondary, now)?;
        let snap = UsageSnapshot {
            five_hour: five,
            weekly: week,
            fetched_at_unix: now,
        };
        if !snap.is_sane() {
            return Err(AppError::ProtocolChange("snapshot validation"));
        }
        Ok(snap)
    }

    fn parse_window(
        &self,
        kind: WindowKind,
        window: &serde_json::Value,
        now: i64,
    ) -> Result<RateWindow, AppError> {
        let expected_seconds = kind.canonical_seconds().unwrap_or(FIVE_HOUR_SECONDS);
        let used_percent = window
            .get("used_percent")
            .and_then(|v| v.as_f64())
            .ok_or(AppError::ProtocolChange("used_percent"))?;
        if !used_percent.is_finite() {
            return Err(AppError::PercentInvalid(kind));
        }
        if !(0.0..=100.0).contains(&used_percent) {
            return Err(AppError::PercentInvalid(kind));
        }
        let limit_seconds_raw = window
            .get("limit_window_seconds")
            .and_then(|v| v.as_i64())
            .ok_or(AppError::ProtocolChange("limit_window_seconds"))?;
        if !(MIN_WINDOW_SECONDS as i64..=MAX_WINDOW_SECONDS as i64).contains(&limit_seconds_raw) {
            return Err(AppError::WindowDurationMismatch {
                kind,
                actual_seconds: limit_seconds_raw,
                expected_seconds,
            });
        }
        let limit_seconds = limit_seconds_raw as u64;
        if !kind.accepts_duration(limit_seconds) {
            return Err(AppError::WindowDurationMismatch {
                kind,
                actual_seconds: limit_seconds_raw,
                expected_seconds,
            });
        }
        let reset_at = window
            .get("reset_at")
            .and_then(|v| v.as_i64())
            .ok_or(AppError::ProtocolChange("reset_at"))?;

        let lower = now - CLOCK_SKEW_SECONDS;
        let upper = now + (limit_seconds as i64) + CLOCK_SKEW_SECONDS;
        if reset_at < lower || reset_at > upper {
            return Err(AppError::ResetInvalid(kind));
        }

        Ok(RateWindow {
            kind,
            used_percent,
            window_seconds: limit_seconds,
            resets_at_unix: reset_at,
        })
    }
}

impl UsageProvider for CodexProvider {
    fn fetch(&self, credentials: &CodexCredentials) -> Result<UsageSnapshot, AppError> {
        if credentials.access_token.expose().is_empty() {
            return Err(AppError::AccessTokenMissing);
        }
        let request = ProviderRequest {
            endpoint: self.endpoint.to_string(),
            bearer_token: credentials.access_token.expose().to_string(),
            account_id: credentials.account_id.clone(),
        };
        let response = self.transport.send(request)?;
        if response.body().len() > MAX_RESPONSE_BYTES {
            return Err(AppError::ResponseTooLarge);
        }
        self.parse(&response)
    }
}

/// Real WinHTTP-backed transport. It deliberately rejects any redirect to
/// prevent the bearer token from leaking across origins.
#[cfg(windows)]
pub struct UreqTransport {
    agent: std::sync::Mutex<ureq::Agent>,
    proxy_source: ProxySource,
}

#[cfg(windows)]
impl UreqTransport {
    pub fn new() -> Result<Self, AppError> {
        Self::with_proxy(ProxyConfiguration::from_environment()?)
    }

    pub fn with_proxy(configuration: ProxyConfiguration) -> Result<Self, AppError> {
        let tls = native_tls::TlsConnector::new()
            .map_err(|_| AppError::Internal("tls connector".into()))?;
        let mut builder = ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(READ_TIMEOUT)
            // Never let a bearer-token request follow an upstream redirect.
            .redirects(0)
            .tls_connector(std::sync::Arc::new(tls));
        if let Some(proxy) = configuration.proxy {
            builder = builder.proxy(proxy);
        }
        Ok(Self {
            agent: std::sync::Mutex::new(builder.build()),
            proxy_source: configuration.source,
        })
    }

    pub fn proxy_source(&self) -> ProxySource {
        self.proxy_source
    }
}

#[cfg(windows)]
impl ProviderTransport for UreqTransport {
    fn send(&self, request: ProviderRequest) -> Result<UsageResponse, AppError> {
        let agent = self
            .agent
            .lock()
            .map_err(|_| AppError::Internal("transport lock".into()))?;
        let mut builder = agent
            .get(&request.endpoint)
            .set("Authorization", &format!("Bearer {}", request.bearer_token))
            .set("User-Agent", USER_AGENT)
            .set("Accept", "application/json");
        if let Some(account) = request.account_id.as_ref() {
            if !account.is_empty() {
                builder = builder.set("ChatGPT-Account-Id", account);
            }
        }
        match builder.call() {
            Ok(resp) => {
                let status = resp.status();
                if status == 401 {
                    return Err(AppError::Unauthorized);
                }
                if status == 403 {
                    return Err(AppError::Forbidden);
                }
                if status == 429 {
                    let retry = resp
                        .header("Retry-After")
                        .and_then(|raw| raw.parse::<u64>().ok())
                        .map(|s| s.clamp(30, 15 * 60));
                    return Err(AppError::RateLimited {
                        retry_after_secs: retry,
                    });
                }
                if (300..400).contains(&status) {
                    return Err(AppError::RedirectRejected);
                }
                if !(200..300).contains(&status) {
                    return Err(AppError::HttpStatus(status));
                }
                let mut body = String::new();
                use std::io::Read;
                resp.into_reader()
                    .take(MAX_RESPONSE_BYTES as u64)
                    .read_to_string(&mut body)
                    .map_err(|_| AppError::ResponseTooLarge)?;
                Ok(UsageResponse::from_raw(body))
            }
            Err(ureq::Error::Status(401, _)) => Err(AppError::Unauthorized),
            Err(ureq::Error::Status(403, _)) => Err(AppError::Forbidden),
            Err(ureq::Error::Status(429, _)) => Err(AppError::RateLimited {
                retry_after_secs: None,
            }),
            Err(ureq::Error::Status(code, _)) if (300..400).contains(&code) => {
                Err(AppError::RedirectRejected)
            }
            Err(ureq::Error::Status(code, _)) => Err(AppError::HttpStatus(code)),
            Err(ureq::Error::Transport(t)) => {
                if t.kind() == ureq::ErrorKind::ConnectionFailed || t.kind() == ureq::ErrorKind::Dns
                {
                    // Do not retain t.to_string(): proxy errors can include
                    // proxy addresses or credentials.
                    Err(AppError::Network)
                } else {
                    Err(AppError::Timeout)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::SecretString;
    use crate::model::WEEKLY_SECONDS;
    use std::sync::Arc;
    use std::sync::Mutex;

    struct FakeTransport {
        recorded: Arc<Mutex<Vec<ProviderRequest>>>,
        bodies: Mutex<std::collections::VecDeque<&'static str>>,
        statuses: Mutex<std::collections::VecDeque<i32>>,
    }

    impl FakeTransport {
        fn new(bodies: Vec<&'static str>, statuses: Vec<i32>) -> Self {
            Self {
                recorded: Arc::new(Mutex::new(Vec::new())),
                bodies: Mutex::new(bodies.into()),
                statuses: Mutex::new(statuses.into()),
            }
        }
    }

    impl ProviderTransport for FakeTransport {
        fn send(&self, request: ProviderRequest) -> Result<UsageResponse, AppError> {
            self.recorded.lock().unwrap().push(request);
            let status = self.statuses.lock().unwrap().pop_front().unwrap_or(200);
            let body = self.bodies.lock().unwrap().pop_front().unwrap_or("{}");
            if status != 200 {
                return Err(status_to_error(status));
            }
            Ok(UsageResponse::from_raw(body.to_string()))
        }
    }

    fn status_to_error(status: i32) -> AppError {
        match status {
            401 => AppError::Unauthorized,
            403 => AppError::Forbidden,
            429 => AppError::RateLimited {
                retry_after_secs: None,
            },
            n if (300..400).contains(&n) => AppError::RedirectRejected,
            n => AppError::HttpStatus(n as u16),
        }
    }

    fn creds() -> CodexCredentials {
        CodexCredentials {
            access_token: SecretString::new("token".into()),
            account_id: Some("acc".into()),
        }
    }

    fn body_with(used_five: f64, secs_five: i64, reset_in: i64) -> String {
        let now = UsageSnapshot::now();
        let payload = serde_json::json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": used_five,
                    "limit_window_seconds": secs_five,
                    "reset_at": now + reset_in,
                },
                "secondary_window": {
                    "used_percent": 15.0,
                    "limit_window_seconds": 604_800_i64,
                    "reset_at": now + 60 * 60 * 24 * 3,
                }
            }
        });
        payload.to_string()
    }

    fn body_5h(used: f64, secs: i64, reset_in: i64) -> String {
        body_with(used, secs, reset_in)
    }

    fn valid_body() -> String {
        body_5h(42.0, 18_000, 60 * 60 + 18 * 60)
    }

    #[test]
    fn successful_response_yields_snapshot() {
        let transport = FakeTransport::new(vec![valid_body().leak()], vec![200]);
        let provider =
            CodexProvider::with_transport("https://example.test/wham/usage", Box::new(transport));
        let snap = provider.fetch(&creds()).expect("snapshot");
        assert!((snap.five_hour.used_percent - 42.0).abs() < 0.001);
        assert_eq!(snap.five_hour.window_seconds, FIVE_HOUR_SECONDS);
        assert_eq!(snap.weekly.window_seconds, WEEKLY_SECONDS);
    }

    #[test]
    fn accepts_dynamic_primary_duration_within_bounds() {
        let body = body_5h(10.0, 6 * 3600, 6 * 3600 - 60);
        let transport = FakeTransport::new(vec![body.leak()], vec![200]);
        let provider = CodexProvider::with_default(Box::new(transport));
        let snap = provider.fetch(&creds()).expect("snapshot");
        assert_eq!(snap.five_hour.window_seconds, 6 * 3600);
        assert_eq!(snap.weekly.window_seconds, WEEKLY_SECONDS);
    }

    #[test]
    fn rejects_primary_duration_below_role_bound() {
        // 30 minutes is below the 1-hour primary bound.
        let body = body_5h(10.0, 30 * 60, 1000);
        let transport = FakeTransport::new(vec![body.leak()], vec![200]);
        let provider = CodexProvider::with_default(Box::new(transport));
        let err = provider.fetch(&creds()).expect_err("must fail");
        assert!(matches!(
            err,
            AppError::WindowDurationMismatch {
                kind: WindowKind::FiveHour,
                actual_seconds: 1800,
                expected_seconds: 18_000,
            }
        ));
    }

    #[test]
    fn rejects_primary_duration_above_role_bound() {
        // 36 hours is above the 24-hour primary bound.
        let body = body_5h(10.0, 36 * 3600, 36 * 3600 - 60);
        let transport = FakeTransport::new(vec![body.leak()], vec![200]);
        let provider = CodexProvider::with_default(Box::new(transport));
        let err = provider.fetch(&creds()).expect_err("must fail");
        assert!(matches!(
            err,
            AppError::WindowDurationMismatch {
                kind: WindowKind::FiveHour,
                actual_seconds: 129_600,
                expected_seconds: 18_000,
            }
        ));
    }

    #[test]
    fn rejects_missing_secondary_window() {
        let now = UsageSnapshot::now();
        let body = format!(
            r#"{{"rate_limit":{{"primary_window":{{"used_percent":30.0,"limit_window_seconds":18000,"reset_at":{five}}}}}}}"#,
            five = now + 60 * 60,
        );
        let transport = FakeTransport::new(vec![body.leak()], vec![200]);
        let provider = CodexProvider::with_default(Box::new(transport));
        assert_eq!(
            provider.fetch(&creds()).expect_err("must fail"),
            AppError::WindowMissing(WindowKind::Weekly),
        );
    }

    #[test]
    fn rejects_invalid_percent() {
        let body = body_5h(150.0, 18_000, 60 * 60);
        let transport = FakeTransport::new(vec![body.leak()], vec![200]);
        let provider = CodexProvider::with_default(Box::new(transport));
        assert_eq!(
            provider.fetch(&creds()).expect_err("must fail"),
            AppError::PercentInvalid(WindowKind::FiveHour),
        );
    }

    #[test]
    fn rejects_redirect() {
        let transport = FakeTransport::new(vec![""], vec![301]);
        let provider = CodexProvider::with_default(Box::new(transport));
        assert_eq!(
            provider.fetch(&creds()).expect_err("must fail"),
            AppError::RedirectRejected
        );
    }

    #[test]
    fn rejects_too_large_response() {
        let huge = "a".repeat(MAX_RESPONSE_BYTES + 1);
        let transport = Box::new(HugeTransport(huge));
        let provider = CodexProvider::with_default(transport);
        let err = provider.fetch(&creds()).expect_err("must fail");
        assert!(matches!(err, AppError::ResponseTooLarge));
    }

    struct HugeTransport(String);
    impl ProviderTransport for HugeTransport {
        fn send(&self, _: ProviderRequest) -> Result<UsageResponse, AppError> {
            Ok(UsageResponse::from_raw(self.0.clone()))
        }
    }

    #[test]
    fn status_codes_map_to_typed_errors() {
        for (status, expected) in [
            (401, AppError::Unauthorized),
            (403, AppError::Forbidden),
            (301, AppError::RedirectRejected),
            (500, AppError::HttpStatus(500)),
        ] {
            let transport = FakeTransport::new(vec![""], vec![status]);
            let provider = CodexProvider::with_default(Box::new(transport));
            let err = provider.fetch(&creds()).expect_err("must fail");
            assert_eq!(err, expected);
        }
    }

    fn proxy_configuration_from(values: &[(&str, &str)]) -> Result<ProxyConfiguration, AppError> {
        let values = values
            .iter()
            .map(|(key, value)| (key.to_string(), std::ffi::OsString::from(value)))
            .collect::<std::collections::HashMap<_, _>>();
        ProxyConfiguration::from_env_lookup(|key| values.get(key).cloned())
    }

    #[test]
    fn proxy_prefers_codexplus_override() {
        let proxy = proxy_configuration_from(&[
            ("http_proxy", "http://http-lower.test:7890"),
            ("HTTPS_PROXY", "http://https.test:7890"),
            ("CODEXPLUS_HTTPS_PROXY", "http://127.0.0.1:7890"),
        ])
        .expect("valid proxy");
        assert_eq!(proxy.source(), ProxySource::CodexplusHttpsProxy);
    }

    #[test]
    fn proxy_prefers_https_before_http_and_skips_empty_values() {
        let proxy = proxy_configuration_from(&[
            ("CODEXPLUS_HTTPS_PROXY", "   "),
            ("HTTPS_PROXY", ""),
            ("https_proxy", "http://https-lower.test:7890"),
            ("HTTP_PROXY", "http://http-upper.test:7890"),
        ])
        .expect("valid proxy");
        assert_eq!(proxy.source(), ProxySource::HttpsProxy);

        let proxy = proxy_configuration_from(&[("http_proxy", "http://127.0.0.1:7890")])
            .expect("valid proxy");
        assert_eq!(proxy.source(), ProxySource::HttpProxy);
    }

    #[test]
    fn proxy_uses_direct_mode_when_no_variable_is_set() {
        let proxy = proxy_configuration_from(&[]).expect("direct mode");
        assert_eq!(proxy.source(), ProxySource::Direct);
        assert!(proxy.proxy.is_none());
    }

    #[test]
    fn direct_policy_ignores_environment_proxy() {
        let proxy = ProxyConfiguration::from_policy(ProxyPolicy::Direct).expect("direct mode");
        assert_eq!(proxy.source(), ProxySource::Direct);
        assert!(proxy.proxy.is_none());
    }

    #[test]
    fn invalid_proxy_never_leaks_url_or_credentials() {
        let secret = "http://alice:s3cret@proxy.example:bad-port";
        let err = match proxy_configuration_from(&[("CODEXPLUS_HTTPS_PROXY", secret)]) {
            Ok(_) => panic!("invalid proxy must fail"),
            Err(err) => err,
        };
        assert_eq!(err, AppError::ProxyConfigurationInvalid);
        assert!(!err.public_message().contains("alice"));
        assert!(!err.public_message().contains("s3cret"));
        assert!(!err.to_string().contains("proxy.example"));
        assert!(!format!("{err:?}").contains("s3cret"));
    }

    #[test]
    fn proxy_labels_do_not_include_configuration_values() {
        assert_eq!(ProxySource::Direct.diagnostic_label(), "direct");
        assert_eq!(
            ProxySource::CodexplusHttpsProxy.diagnostic_label(),
            "CODEXPLUS_HTTPS_PROXY"
        );
        assert_eq!(ProxySource::HttpsProxy.diagnostic_label(), "HTTPS_PROXY");
        assert_eq!(ProxySource::HttpProxy.diagnostic_label(), "HTTP_PROXY");
    }

    #[test]
    fn retry_class_distinguishes_categories() {
        assert_eq!(
            AppError::Unauthorized.retry_class(),
            crate::error::RetryClass::AuthFailure
        );
        assert_eq!(
            AppError::Timeout.retry_class(),
            crate::error::RetryClass::Network
        );
        assert_eq!(
            AppError::Network.retry_class(),
            crate::error::RetryClass::Network
        );
        assert_eq!(
            AppError::ProxyConfigurationInvalid.retry_class(),
            crate::error::RetryClass::Network
        );
        assert_eq!(
            AppError::RedirectRejected.retry_class(),
            crate::error::RetryClass::Protocol
        );
    }
}
