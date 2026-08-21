#![cfg_attr(windows, windows_subsystem = "windows")]

use std::env;
use std::process::ExitCode;

#[cfg(windows)]
fn main() -> ExitCode {
    let mut args = env::args();
    let _ = args.next();
    if args.next().as_deref() == Some("--uia-helper") {
        let parent_pid = args.next().and_then(|value| value.parse().ok());
        let code = codexplus::ui::review_automation_ipc::run_helper(parent_pid);
        return ExitCode::from(code as u8);
    }
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
