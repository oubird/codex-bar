#![cfg_attr(windows, windows_subsystem = "windows")]

use std::env;
use std::process::ExitCode;

#[cfg(windows)]
fn main() -> ExitCode {
    if env::args().any(|arg| arg == "--diagnose") {
        let code = codexplus::diagnose::run();
        return ExitCode::from(code as u8);
    }
    codexplus::app::run()
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("codexplus only supports Windows; this binary will not start here.");
    ExitCode::from(1)
}
