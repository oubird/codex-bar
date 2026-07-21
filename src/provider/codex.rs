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
    RateWindow, UsageSnapshot, WindowKind, CLOCK_SKEW_SECONDS, FIVE_HOUR_SECONDS, MAX_WINDOW_SECONDS,
    MIN_WINDOW_SECONDS, WEEKLY_SECONDS,
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
        Self { endpoint, transport }
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
        let five = self.parse_window(WindowKind::FiveHour, primary, FIVE_HOUR_SECONDS, now)?;
        let week = self.parse_window(WindowKind::Weekly, secondary, WEEKLY_SECONDS, now)?;
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
        expected_seconds: u64,
        now: i64,
    ) -> Result<RateWindow, AppError> {
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
        let limit_seconds = window
            .get("limit_window_seconds")
            .and_then(|v| v.as_i64())
            .ok_or(AppError::ProtocolChange("limit_window_seconds"))?;
        if !(MIN_WINDOW_SECONDS as i64..=MAX_WINDOW_SECONDS as i64).contains(&limit_seconds) {
            return Err(AppError::WindowDurationMismatch {
                kind,
                actual_seconds: limit_seconds,
            });
        }
        let reset_at = window
            .get("reset_at")
            .and_then(|v| v.as_i64())
            .ok_or(AppError::ProtocolChange("reset_at"))?;

        if (limit_seconds as u64) != expected_seconds {
            return Err(AppError::WindowDurationMismatch {
                kind,
                actual_seconds: limit_seconds,
            });
        }

        let lower = now - CLOCK_SKEW_SECONDS;
        let upper = now + (expected_seconds as i64) + CLOCK_SKEW_SECONDS;
        if reset_at < lower || reset_at > upper {
            return Err(AppError::ResetInvalid(kind));
        }

        Ok(RateWindow {
            kind,
            used_percent,
            window_seconds: expected_seconds,
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
}

#[cfg(windows)]
impl UreqTransport {
    pub fn new() -> Self {
        let tls = native_tls::TlsConnector::new().expect("native-tls connector");
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(READ_TIMEOUT)
            .redirects(0)
            .tls_connector(std::sync::Arc::new(tls))
            .build();
        Self { agent: std::sync::Mutex::new(agent) }
    }
}

#[cfg(windows)]
impl ProviderTransport for UreqTransport {
    fn send(&self, request: ProviderRequest) -> Result<UsageResponse, AppError> {
        let agent = self.agent.lock().map_err(|_| AppError::Internal("transport lock".into()))?;
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
                    return Err(AppError::RateLimited { retry_after_secs: retry });
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
            Err(ureq::Error::Status(code, _)) if code == 401 => Err(AppError::Unauthorized),
            Err(ureq::Error::Status(code, _)) if code == 403 => Err(AppError::Forbidden),
            Err(ureq::Error::Status(code, _)) if code == 429 => {
                Err(AppError::RateLimited { retry_after_secs: None })
            }
            Err(ureq::Error::Status(code, _)) if (300..400).contains(&code) => {
                Err(AppError::RedirectRejected)
            }
            Err(ureq::Error::Status(code, _)) => Err(AppError::HttpStatus(code)),
            Err(ureq::Error::Transport(t)) => {
                if t.kind() == ureq::ErrorKind::ConnectionFailed
                    || t.kind() == ureq::ErrorKind::Dns
                {
                    Err(AppError::Network(t.to_string()))
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
            429 => AppError::RateLimited { retry_after_secs: None },
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
        let provider = CodexProvider::with_transport(
            "https://example.test/wham/usage",
            Box::new(transport),
        );
        let snap = provider.fetch(&creds()).expect("snapshot");
        assert!((snap.five_hour.used_percent - 42.0).abs() < 0.001);
        assert_eq!(snap.five_hour.window_seconds, FIVE_HOUR_SECONDS);
        assert_eq!(snap.weekly.window_seconds, WEEKLY_SECONDS);
    }

    #[test]
    fn rejects_incorrect_primary_duration() {
        let body = body_5h(10.0, 3600, 1000);
        let transport = FakeTransport::new(vec![body.leak()], vec![200]);
        let provider = CodexProvider::with_default(Box::new(transport));
        let err = provider.fetch(&creds()).expect_err("must fail");
        assert!(matches!(
            err,
            AppError::WindowDurationMismatch {
                kind: WindowKind::FiveHour,
                actual_seconds: 3600
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

    #[test]
    fn retry_class_distinguishes_categories() {
        assert_eq!(AppError::Unauthorized.retry_class(), crate::error::RetryClass::AuthFailure);
        assert_eq!(AppError::Timeout.retry_class(), crate::error::RetryClass::Network);
        assert_eq!(AppError::RedirectRejected.retry_class(), crate::error::RetryClass::Protocol);
    }
}
