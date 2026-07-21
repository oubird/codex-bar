//! CodexPlus — Windows title-bar overlay that surfaces Codex 5-hour and
//! weekly usage limits.
//!
//! The library is split into small modules so that pure logic (models,
//! formatting, storage, provider parsing) can be exercised without bringing
//! in Win32 APIs.

pub mod model;
pub mod error;
pub mod formatting;
pub mod credentials;
pub mod provider;
pub mod storage;

#[cfg(windows)]
pub mod ui;
#[cfg(windows)]
pub mod app;
#[cfg(windows)]
pub mod diagnose;