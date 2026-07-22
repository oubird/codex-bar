//! Identify the Codex Windows desktop app and track its top-level window.
//!
//! The Codex desktop product line does not currently expose a stable CLI
//! flag we can rely on, so we identify its primary top-level window by:
//!
//! 1. Excluding terminal hosts (`WindowsTerminal.exe`, `pwsh.exe`,
//!    `powershell.exe`, `cmd.exe`, `Codex.exe` running inside WSL/tmux/
//!    iTerm-style hosts).
//! 2. Requiring the binary basename to match a deliberately conservative
//!    allowlist of Codex desktop binaries. Operators can extend that
//!    allowlist via `CODEXPLUS_TARGET_BIN` (comma-separated exe basenames).
//! 3. Filtering to the currently foreground, *root* window of that
//!    process and ignoring owner windows.

#![cfg(windows)]

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::IsIconic;

use super::native;

/// Process-level identity collected from a live HWND.
#[derive(Debug, Clone)]
pub struct WindowIdentity {
    pub hwnd: isize,
    pub pid: u32,
    pub exe_basename: String,
    pub exe_path: String,
    pub class_name: String,
    pub title: String,
}

/// Snapshot used by diagnostics — never includes credentials.
#[derive(Debug, Clone)]
pub struct TargetDiagnostics {
    pub active: Option<WindowIdentity>,
    pub candidates: Vec<WindowIdentity>,
}

/// Allowlist of Codex desktop app exe basenames.
///
/// Populated conservatively so the MVP does not accidentally attach to
/// random WebKit/Electron renderers named `app.exe`. Operators may extend
/// this list through the `CODEXPLUS_TARGET_BIN` environment variable.
/// Effective executable-basename allowlist, including any user override.
pub fn target_executable_allowlist() -> Vec<String> {
    fn canonicalise(value: &str) -> String {
        value
            .trim()
            .trim_end_matches(".exe")
            .trim_end_matches(".EXE")
            .to_ascii_lowercase()
    }

    // The Microsoft Store Codex desktop build currently hosts its top-level
    // window in ChatGPT.exe; retain the explicit Codex names for standalone
    // distributions and allow operators to extend this via CODEXPLUS_TARGET_BIN.
    let defaults = ["codex", "codex desktop", "chatgpt"];
    let env = std::env::var("CODEXPLUS_TARGET_BIN")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(canonicalise)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    defaults.into_iter().map(canonicalise).chain(env).collect()
}

/// Process basenames that should *never* be selected as the Codex host,
/// regardless of any other matching rule. These all spawn web contents
/// with the same window class but are not the Codex desktop product.
fn disallow_exe_basename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "windowsterminal"
            | "windowsterminal.exe"
            | "powershell"
            | "powershell.exe"
            | "pwsh"
            | "pwsh.exe"
            | "cmd"
            | "cmd.exe"
            | "wt"
            | "wt.exe"
            | "msedge"
            | "chrome"
            | "firefox"
            | "code"
    )
}

pub fn inspect_window(hwnd: windows::Win32::Foundation::HWND) -> Option<WindowIdentity> {
    if hwnd.is_invalid() {
        return None;
    }
    let (_, pid) = native::window_thread_process_id(hwnd);
    if pid == 0 {
        return None;
    }
    let exe_basename = process_basename(pid)?;
    let exe_path = process_image_path(pid).unwrap_or_default();
    let class_name = native::class_name(hwnd);
    let title = window_title(hwnd).unwrap_or_default();
    Some(WindowIdentity {
        hwnd: hwnd.0 as isize,
        pid,
        exe_basename,
        exe_path,
        class_name,
        title,
    })
}

fn window_title(hwnd: windows::Win32::Foundation::HWND) -> Option<String> {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowTextW;
    unsafe {
        let mut buf = [0u16; 256];
        let len = GetWindowTextW(hwnd, &mut buf);
        if len == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

fn process_basename(pid: u32) -> Option<String> {
    let path = process_image_path(pid)?;
    Some(
        std::path::Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
    )
}

fn process_image_path(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        if handle.is_invalid() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(handle);
        result.ok()?;
        let os = OsString::from_wide(&buf[..size as usize]);
        Some(os.to_string_lossy().into_owned())
    }
}

/// Decide whether the given candidate is the Codex desktop app process.
pub fn is_codex_desktop(candidate: &WindowIdentity) -> bool {
    if disallow_exe_basename(&candidate.exe_basename) {
        return false;
    }
    let stripped = candidate
        .exe_basename
        .trim_end_matches(".exe")
        .trim_end_matches(".EXE")
        .to_ascii_lowercase();
    target_executable_allowlist()
        .iter()
        .any(|allowed| allowed == &stripped)
}

/// Is the target window currently visible, restored or maximised, and has
/// a usable window rectangle? Full-screen/borderless windows are handled by
/// title-bar placement geometry rather than being rejected pre-emptively.
pub fn is_target_displayable(hwnd: windows::Win32::Foundation::HWND) -> bool {
    if hwnd.is_invalid() {
        return false;
    }
    if !native::is_window_visible(hwnd) {
        return false;
    }
    unsafe {
        if IsIconic(hwnd).as_bool() {
            return false;
        }
    }
    if native::window_rect(hwnd).is_none_or(|r| r.right - r.left <= 0 || r.bottom - r.top <= 0) {
        return false;
    }
    true
}

/// Return the foreground codex-window, if any.
pub fn foreground_codex_window() -> Option<WindowIdentity> {
    let candidates = collect_codex_candidates();
    let fg_root = native::root_owner(native::foreground_window()).0 as isize;
    candidates
        .iter()
        .find(|id| id.hwnd == fg_root)
        .cloned()
        .or_else(|| {
            // No candidate is the literal foreground root; fall back to the
            // topmost codex window owned by the foreground thread/process.
            let (_, fg_pid) = native::window_thread_process_id(native::foreground_window());
            candidates.into_iter().find(|id| id.pid == fg_pid)
        })
}

/// Inspect the current foreground root window without applying the Codex allowlist.
pub fn foreground_window_identity() -> Option<WindowIdentity> {
    inspect_window(native::root_owner(native::foreground_window()))
}

pub fn collect_codex_candidates() -> Vec<WindowIdentity> {
    native::enumerate_top_level()
        .into_iter()
        .filter_map(inspect_window)
        .filter(is_codex_desktop)
        .collect()
}

pub fn diagnostics() -> TargetDiagnostics {
    let candidates = native::enumerate_top_level()
        .into_iter()
        .filter_map(inspect_window)
        .collect::<Vec<_>>();
    let active = foreground_codex_window();
    TargetDiagnostics { active, candidates }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str) -> WindowIdentity {
        WindowIdentity {
            hwnd: 7,
            pid: 42,
            exe_basename: name.into(),
            exe_path: String::new(),
            class_name: String::new(),
            title: String::new(),
        }
    }

    #[test]
    fn allowlist_matches_codex_basenames() {
        let mut list = target_executable_allowlist();
        // ensure no duplicates from env misuse
        list.sort();
        list.dedup();
        assert!(is_codex_desktop(&candidate("codex.exe")));
        assert!(is_codex_desktop(&candidate("Codex Desktop.exe")));
        assert!(!is_codex_desktop(&candidate("code.exe")));
    }

    #[test]
    fn disallow_list_blocks_terminals() {
        for exe in [
            "WindowsTerminal.exe",
            "powershell.exe",
            "pwsh.exe",
            "cmd.exe",
            "wt.exe",
            "msedge.exe",
        ] {
            assert!(!is_codex_desktop(&candidate(exe)), "should reject {exe}");
        }
    }
}
