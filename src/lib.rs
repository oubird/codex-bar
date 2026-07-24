//! CodexPlus — Windows title-bar overlay that surfaces Codex 5-hour and
//! weekly usage limits.
//!
//! The library is split into small modules so that pure logic (models,
//! formatting, storage, provider parsing) can be exercised without bringing
//! in Win32 APIs.

pub mod credentials;
pub mod error;
pub mod formatting;
pub mod localization;
pub mod model;
pub mod provider;
pub mod settings;
pub mod storage;

#[cfg(windows)]
pub mod app;
#[cfg(windows)]
pub mod diagnose;
#[cfg(windows)]
pub mod ui;
