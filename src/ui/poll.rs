//! Polling-thread plumbing that feeds [`crate::ui::native::run_app`].

#![cfg(windows)]

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::credentials::{credential_signature, read_credentials, resolve_auth_path};
use crate::error::{AppError, RetryClass};
use crate::model::UsageSnapshot;
use crate::provider::{CodexProvider, ProxyPolicy, UsageProvider};

#[cfg(windows)]
use crate::provider::UreqTransport;

pub enum PollOutcome {
    Success(UsageSnapshot),
    Failure(AppError),
}

#[derive(Clone)]
pub struct PollSender {
    sender: mpsc::Sender<PollOutcome>,
}

impl PollSender {
    pub fn request_refresh(&self, policy: ProxyPolicy) {
        let sender = self.sender.clone();
        thread::spawn(move || {
            let outcome = run_poll(policy);
            let _ = sender.send(outcome);
        });
    }
}

#[cfg(windows)]
pub fn start() -> (PollSender, mpsc::Receiver<PollOutcome>) {
    let (tx, rx) = mpsc::channel();
    let ctrl = PollSender { sender: tx };
    (ctrl, rx)
}

#[cfg(windows)]
fn run_poll(policy: ProxyPolicy) -> PollOutcome {
    let proxy = match crate::provider::ProxyConfiguration::from_policy(policy) {
        Ok(proxy) => proxy,
        Err(err) => return PollOutcome::Failure(err),
    };
    let transport = match UreqTransport::with_proxy(proxy) {
        Ok(transport) => transport,
        Err(err) => return PollOutcome::Failure(err),
    };
    let provider = CodexProvider::with_default(Box::new(transport));
    match poll_once(&provider) {
        Ok(snap) => PollOutcome::Success(snap),
        Err(err) => PollOutcome::Failure(err),
    }
}

fn poll_once(provider: &dyn UsageProvider) -> Result<UsageSnapshot, AppError> {
    let path = resolve_auth_path()?;
    let creds = read_credentials(&path)?;
    provider.fetch(&creds)
}

pub fn retry_class_delay(class: RetryClass) -> Duration {
    match class {
        RetryClass::Network => Duration::from_secs(30),
        RetryClass::RateLimited => Duration::from_secs(120),
        RetryClass::AuthFailure => Duration::from_secs(0),
        RetryClass::Protocol => Duration::from_secs(300),
        RetryClass::Cache => Duration::from_secs(5),
        RetryClass::NoRetry => Duration::from_secs(900),
    }
}

pub fn auth_signature_summary() -> String {
    match resolve_auth_path() {
        Ok(path) => format!("{:?}", credential_signature(&path)),
        Err(_) => "unset".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_class_delays_are_reasonable() {
        assert!(retry_class_delay(RetryClass::Network).as_secs() >= 10);
        assert!(retry_class_delay(RetryClass::RateLimited).as_secs() >= 60);
    }
}
