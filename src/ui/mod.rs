//! Windows UI surface module.

pub mod app_state;
pub mod native;
pub mod poll;
#[cfg(windows)]
pub mod prompt_menu;
pub mod render;
#[cfg(windows)]
pub mod review_automation;
#[cfg(windows)]
pub mod review_button;
pub mod settings;
pub mod target_window;

#[cfg(windows)]
pub mod overlay;
#[cfg(windows)]
mod tests;
#[cfg(windows)]
pub mod tray;
