//! Windows UI surface module.

pub mod native;
pub mod poll;
pub mod render;
pub mod target_window;

#[cfg(windows)]
pub mod overlay;
#[cfg(windows)]
pub mod tray;
#[cfg(windows)]
mod tests;
