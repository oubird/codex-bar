//! Native popup menu for user-configured quick prompts.

#![cfg(windows)]

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, TrackPopupMenuEx, MF_STRING, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD,
};

const FIRST_PROMPT_ID: usize = 41_000;
const MAX_MENU_LABEL_CHARS: usize = 64;

pub fn select_prompt(owner: HWND, anchor: RECT, prompts: &[String]) -> Option<usize> {
    if prompts.is_empty() {
        return None;
    }
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        for (index, prompt) in prompts.iter().enumerate() {
            let label = menu_label(index, prompt);
            let wide = wide(&label);
            if AppendMenuW(
                menu,
                MF_STRING,
                FIRST_PROMPT_ID + index,
                PCWSTR(wide.as_ptr()),
            )
            .is_err()
            {
                let _ = DestroyMenu(menu);
                return None;
            }
        }
        let selected = TrackPopupMenuEx(
            menu,
            (TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RETURNCMD | TPM_NONOTIFY).0,
            anchor.left,
            anchor.top,
            owner,
            None,
        )
        .0 as usize;
        let _ = DestroyMenu(menu);
        selected
            .checked_sub(FIRST_PROMPT_ID)
            .filter(|index| *index < prompts.len())
    }
}

fn menu_label(index: usize, prompt: &str) -> String {
    let trimmed = prompt.trim();
    let mut preview = trimmed
        .chars()
        .take(MAX_MENU_LABEL_CHARS)
        .collect::<String>();
    if trimmed.chars().count() > MAX_MENU_LABEL_CHARS {
        preview.push('…');
    }
    format!(
        "{}. {}",
        index + 1,
        preview.replace('&', "&&").replace('\t', " ")
    )
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_labels_escape_mnemonics_and_keep_order() {
        assert_eq!(menu_label(1, "A&B"), "2. A&&B");
    }

    #[test]
    fn menu_labels_truncate_only_the_visible_copy() {
        let prompt = "甲".repeat(MAX_MENU_LABEL_CHARS + 10);
        let label = menu_label(0, &prompt);
        assert!(label.ends_with('…'));
        assert_eq!(prompt.chars().count(), MAX_MENU_LABEL_CHARS + 10);
    }
}
