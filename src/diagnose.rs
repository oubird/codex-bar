//! `--diagnose` mode — runs a one-shot probe and writes a human-readable
//! report to a file in `%TEMP%`. The report covers:
//!
//! * Where the Codex `auth.json` was looked up, whether it exists, its
//!   size and modification time. **No token content is ever written.**
//! * Whether the JSON parsed and what fields were found
//!   (`access_token` present, `account_id` present).
//! * Whether the `wham/usage` request was sent, what status came back,
//!   and how the response was parsed (or why it failed).
//! * Whether the last-good cache was loaded / written.
//!
//! The mode is intentionally side-effect free on the Codex session: it
//! never modifies `auth.json`, never calls Codex CLI, and never follows
//! HTTP redirects.

#![cfg(windows)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::credentials::{credential_signature, read_credentials, resolve_auth_path};
use crate::formatting;
use crate::model::ViewState;
use crate::provider::{
    CodexProvider, ProxyConfiguration, ProxySource, UreqTransport, UsageProvider,
};
use crate::settings;
use crate::storage;
use crate::ui::target_window;

const LOG_PREFIX: &str = "codexplus-diagnose";

pub fn run() -> i32 {
    let log_path = log_path();
    let mut report = String::new();
    writeln!(report, "CodexPlus diagnose report").ok();
    writeln!(report, "========================").ok();
    writeln!(report, "Generated: {}", crate::model::UsageSnapshot::now()).ok();
    writeln!(report, "Log file:  {}", log_path.display()).ok();
    writeln!(report).ok();

    report_auth(&mut report);
    report_cache_load(&mut report);
    let outcome = report_live_request(&mut report);
    if let Some(snapshot) = outcome.as_ref() {
        report_cache_write(&mut report, snapshot);
    }
    report_state(&mut report, outcome.as_ref());
    report_target_window(&mut report);

    let write_result = write_log(&log_path, &report);
    match write_result {
        Ok(()) => show_summary(&log_path, "诊断日志已生成"),
        Err(err) => show_summary(&log_path, &format!("诊断日志写入失败：{err}")),
    }
    0
}

fn log_path() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = format!("{LOG_PREFIX}-{stamp}.log");
    std::env::temp_dir().join(name)
}

fn write_log(path: &Path, contents: &str) -> std::io::Result<()> {
    fs::write(path, contents)
}

fn report_auth(report: &mut String) {
    writeln!(report, "1. Credential lookup").ok();
    writeln!(report, "--------------------").ok();
    match resolve_auth_path() {
        Ok(path) => {
            writeln!(report, "  Resolved path : {}", path.display()).ok();
            let sig = credential_signature(&path);
            writeln!(report, "  Exists        : {}", sig.exists).ok();
            writeln!(report, "  Size (bytes)  : {}", sig.size).ok();
            if let Some(modified) = sig.modified_unix {
                writeln!(report, "  Modified (u)  : {modified}").ok();
            } else {
                writeln!(report, "  Modified (u)  : <unknown>").ok();
            }
            match read_credentials(&path) {
                Ok(_) => {
                    writeln!(report, "  JSON parsed   : yes").ok();
                    // Do not reveal a token, its length, account ID, or
                    // even an account-presence bit in the diagnostic file.
                    writeln!(report, "  Session fields: present").ok();
                }
                Err(err) => {
                    writeln!(report, "  JSON parsed   : no").ok();
                    writeln!(report, "  Error         : {}", err.public_message()).ok();
                }
            }
        }
        Err(err) => {
            writeln!(report, "  Resolved path : <unavailable>").ok();
            writeln!(report, "  Error         : {}", err.public_message()).ok();
        }
    }
    writeln!(report).ok();
}

fn report_cache_load(report: &mut String) {
    writeln!(report, "2. Last-good cache load").ok();
    writeln!(report, "----------------------").ok();
    match storage::cache_path() {
        Some(path) => {
            writeln!(report, "  Cache path    : {}", path.display()).ok();
            match storage::load() {
                Some(snapshot) => {
                    writeln!(
                        report,
                        "  Loaded        : yes (fetched_at = {})",
                        snapshot.fetched_at_unix
                    )
                    .ok();
                    writeln!(
                        report,
                        "  5h used       : {:.2}%",
                        snapshot.five_hour.used_percent
                    )
                    .ok();
                    writeln!(
                        report,
                        "  weekly used   : {:.2}%",
                        snapshot.weekly.used_percent
                    )
                    .ok();
                }
                None => {
                    writeln!(report, "  Loaded        : <no cache file>").ok();
                }
            }
        }
        None => {
            writeln!(report, "  Cache path    : <unavailable>").ok();
        }
    }
    writeln!(report).ok();
}

fn report_live_request(report: &mut String) -> Option<crate::model::UsageSnapshot> {
    writeln!(report, "3. Live request to wham/usage").ok();
    writeln!(report, "----------------------------").ok();
    let path = match resolve_auth_path() {
        Ok(p) => p,
        Err(err) => {
            writeln!(report, "  Skipped: {}", err.public_message()).ok();
            return None;
        }
    };
    let creds = match read_credentials(&path) {
        Ok(c) => c,
        Err(err) => {
            writeln!(report, "  Skipped: {}", err.public_message()).ok();
            return None;
        }
    };
    let proxy_configuration = match ProxyConfiguration::from_policy(settings::load().proxy_policy())
    {
        Ok(configuration) => configuration,
        Err(err) => {
            writeln!(report, "  Proxy mode    : invalid configuration").ok();
            writeln!(report, "  HTTP response : <error>").ok();
            writeln!(report, "  Detail        : {}", err.diagnostic_detail()).ok();
            writeln!(report, "  Retry class   : {:?}", err.retry_class()).ok();
            return None;
        }
    };
    writeln!(
        report,
        "  Proxy mode    : {}",
        format_proxy_mode(proxy_configuration.source())
    )
    .ok();
    let transport = match UreqTransport::with_proxy(proxy_configuration) {
        Ok(transport) => transport,
        Err(err) => {
            writeln!(report, "  HTTP response : <error>").ok();
            writeln!(report, "  Detail        : {}", err.diagnostic_detail()).ok();
            writeln!(report, "  Retry class   : {:?}", err.retry_class()).ok();
            return None;
        }
    };
    let provider = CodexProvider::with_default(Box::new(transport));
    match provider.fetch(&creds) {
        Ok(snapshot) => {
            writeln!(
                report,
                "  HTTP response : 200 (parsed {})",
                if snapshot.is_sane() {
                    "OK"
                } else {
                    "but insane!"
                }
            )
            .ok();
            writeln!(
                report,
                "  5h used          : {:.2}%",
                snapshot.five_hour.used_percent
            )
            .ok();
            writeln!(
                report,
                "  5h duration      : {} seconds",
                snapshot.five_hour.window_seconds
            )
            .ok();
            writeln!(
                report,
                "  weekly used      : {:.2}%",
                snapshot.weekly.used_percent
            )
            .ok();
            writeln!(
                report,
                "  weekly duration  : {} seconds",
                snapshot.weekly.window_seconds
            )
            .ok();
            writeln!(
                report,
                "  5h resets_at     : {}",
                snapshot.five_hour.resets_at_unix
            )
            .ok();
            writeln!(
                report,
                "  weekly resets_at : {}",
                snapshot.weekly.resets_at_unix
            )
            .ok();
            Some(snapshot)
        }
        Err(err) => {
            writeln!(report, "  HTTP response : <error>").ok();
            writeln!(report, "  Detail        : {}", err.diagnostic_detail()).ok();
            writeln!(report, "  Retry class   : {:?}", err.retry_class()).ok();
            None
        }
    }
}

fn format_proxy_mode(source: ProxySource) -> &'static str {
    source.diagnostic_label()
}

fn report_cache_write(report: &mut String, snapshot: &crate::model::UsageSnapshot) {
    writeln!(report).ok();
    writeln!(report, "4. Last-good cache write").ok();
    writeln!(report, "------------------------").ok();
    match storage::save(snapshot) {
        Ok(()) => {
            writeln!(report, "  Saved         : yes").ok();
            if let Some(path) = storage::cache_path() {
                writeln!(report, "  Path          : {}", path.display()).ok();
            }
        }
        Err(err) => {
            writeln!(report, "  Saved         : no").ok();
            writeln!(report, "  Error         : {}", err.public_message()).ok();
        }
    }
}

fn report_state(report: &mut String, snapshot: Option<&crate::model::UsageSnapshot>) {
    writeln!(report).ok();
    writeln!(report, "5. ViewState summary").ok();
    writeln!(report, "-------------------").ok();
    let now = crate::model::UsageSnapshot::now();
    let state = match snapshot {
        Some(snap) => ViewState::Ready {
            snapshot: snap.clone(),
        },
        None => match storage::load() {
            Some(snap) => ViewState::Stale {
                snapshot: snap,
                reason: "live request failed; using cached snapshot",
            },
            None => ViewState::Error {
                cached: None,
                reason: "no live data and no cached snapshot",
            },
        },
    };
    writeln!(
        report,
        "  {}",
        formatting::format_diagnostic_summary(&state, now)
    )
    .ok();
}

fn report_target_window(report: &mut String) {
    writeln!(report).ok();
    writeln!(report, "6. Overlay target detection").ok();
    writeln!(report, "---------------------------").ok();
    let allowlist = target_window::target_executable_allowlist();
    writeln!(report, "  Allowed basenames : {}", allowlist.join(", ")).ok();

    match target_window::foreground_window_identity() {
        Some(window) => {
            writeln!(report, "  Foreground exe   : {}", window.exe_basename).ok();
            writeln!(report, "  Foreground class : {}", window.class_name).ok();
        }
        None => {
            writeln!(report, "  Foreground window: <unavailable>").ok();
        }
    }

    match target_window::foreground_codex_window() {
        Some(target) => {
            let hwnd = windows::Win32::Foundation::HWND(target.hwnd as *mut _);
            writeln!(report, "  Target matched   : yes ({})", target.exe_basename).ok();
            writeln!(
                report,
                "  Target eligible  : {}",
                target_window::is_target_displayable(hwnd)
            )
            .ok();
        }
        None => {
            writeln!(report, "  Target matched   : no").ok();
            writeln!(report, "  Target eligible  : no").ok();
        }
    }
}

fn show_summary(log_path: &Path, status: &str) {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};

    let message = format!(
        "CodexPlus 诊断已完成。\n\n状态：{status}\n日志：{}",
        log_path.display()
    );
    let wide_message: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
    let wide_title: Vec<u16> = "CodexPlus 诊断"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        MessageBoxW(
            HWND(std::ptr::null_mut()),
            PCWSTR(wide_message.as_ptr()),
            PCWSTR(wide_title.as_ptr()),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}
