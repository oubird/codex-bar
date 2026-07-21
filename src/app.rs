//! Application lifecycle and message-pump glue.

use std::process::ExitCode;

#[cfg(not(windows))]
pub fn run() -> ExitCode {
    ExitCode::from(1)
}

#[cfg(windows)]
pub fn run() -> ExitCode {
    match crate::ui::native::run_app() {
        Ok(()) => ExitCode::from(0),
        Err(_) => ExitCode::from(1),
    }
}
