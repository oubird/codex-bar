//! UI-thread state transitions for the resident application.
//!
//! Keeping these transitions independent of HWNDs lets the host window be
//! small and makes cache/poll behaviour deterministic in tests.

use crate::error::{AppError, RetryClass};
use crate::model::{UsageSnapshot, ViewState};
use crate::settings::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshDisposition {
    Success,
    RetryAfterSeconds(u32),
    WaitForCredentialChange,
}

pub struct AppState {
    pub settings: Settings,
    pub state: ViewState,
    pub last_good: Option<UsageSnapshot>,
    pub refresh_in_flight: bool,
    pub pending_manual_refresh: bool,
}

impl AppState {
    pub fn new(settings: Settings, cached: Option<UsageSnapshot>) -> Self {
        Self {
            settings,
            state: ViewState::Loading {
                cached: cached.clone(),
            },
            last_good: cached,
            refresh_in_flight: false,
            pending_manual_refresh: false,
        }
    }

    pub fn begin_refresh(&mut self, manual: bool) -> bool {
        if self.refresh_in_flight {
            self.pending_manual_refresh |= manual;
            return false;
        }
        self.refresh_in_flight = true;
        true
    }

    pub fn apply_success(&mut self, snapshot: UsageSnapshot) -> RefreshDisposition {
        self.last_good = Some(snapshot.clone());
        self.state = ViewState::Ready { snapshot };
        self.refresh_in_flight = false;
        RefreshDisposition::Success
    }

    pub fn apply_failure(&mut self, error: AppError) -> RefreshDisposition {
        self.refresh_in_flight = false;
        let reason = reason_for(&error);
        self.state = match self.last_good.clone() {
            Some(snapshot) => ViewState::Stale { snapshot, reason },
            None => ViewState::Error {
                cached: None,
                reason,
            },
        };
        match error {
            AppError::RateLimited {
                retry_after_secs: Some(seconds),
            } => RefreshDisposition::RetryAfterSeconds(seconds.clamp(30, 900) as u32),
            AppError::Unauthorized
            | AppError::Forbidden
            | AppError::AuthFileMissing
            | AppError::AuthFileInvalid
            | AppError::AccessTokenMissing => RefreshDisposition::WaitForCredentialChange,
            _ => RefreshDisposition::RetryAfterSeconds(delay_for(error.retry_class())),
        }
    }

    pub fn take_pending_manual_refresh(&mut self) -> bool {
        std::mem::take(&mut self.pending_manual_refresh)
    }
}

fn delay_for(class: RetryClass) -> u32 {
    match class {
        RetryClass::Network => 30,
        RetryClass::RateLimited => 120,
        RetryClass::AuthFailure => 900,
        RetryClass::Protocol => 300,
        RetryClass::Cache => 60,
        RetryClass::NoRetry => 900,
    }
}

fn reason_for(error: &AppError) -> &'static str {
    match error {
        AppError::Unauthorized | AppError::Forbidden => "auth",
        AppError::Network | AppError::Timeout => "network",
        AppError::ProxyConfigurationInvalid | AppError::CustomProxyUnavailable => "proxy",
        AppError::RateLimited { .. } => "rate limited",
        AppError::ProtocolChange(_)
        | AppError::WindowMissing(_)
        | AppError::WindowDurationMismatch { .. }
        | AppError::PercentInvalid(_)
        | AppError::ResetInvalid(_) => "protocol",
        _ => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RateWindow, WindowKind, FIVE_HOUR_SECONDS, WEEKLY_SECONDS};

    fn snapshot() -> UsageSnapshot {
        let now = UsageSnapshot::now();
        UsageSnapshot {
            five_hour: Some(RateWindow {
                kind: WindowKind::FiveHour,
                used_percent: 1.0,
                window_seconds: FIVE_HOUR_SECONDS,
                resets_at_unix: now + 3600,
            }),
            weekly: Some(RateWindow {
                kind: WindowKind::Weekly,
                used_percent: 2.0,
                window_seconds: WEEKLY_SECONDS,
                resets_at_unix: now + 7200,
            }),
            fetched_at_unix: now,
        }
    }

    #[test]
    fn failed_refresh_keeps_last_good_as_stale() {
        let mut state = AppState::new(Settings::default(), Some(snapshot()));
        state.begin_refresh(false);
        assert_eq!(
            state.apply_failure(AppError::Network),
            RefreshDisposition::RetryAfterSeconds(30)
        );
        assert!(matches!(
            state.state,
            ViewState::Stale {
                reason: "network",
                ..
            }
        ));
    }

    #[test]
    fn manual_refresh_is_coalesced_while_busy() {
        let mut state = AppState::new(Settings::default(), None);
        assert!(state.begin_refresh(false));
        assert!(!state.begin_refresh(true));
        assert!(state.take_pending_manual_refresh());
        assert!(!state.take_pending_manual_refresh());
    }
}
