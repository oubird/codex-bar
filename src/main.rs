#![cfg_attr(windows, windows_subsystem = "windows")]

use std::process::ExitCode;

#[cfg(windows)]
fn main() -> ExitCode {
    codexplus::app::run()
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("codexplus only supports Windows; this binary will not start here.");
    ExitCode::from(1)
}
