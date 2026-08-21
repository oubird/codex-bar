//! Safe UI Automation bridge for Codex's built-in uncommitted-changes review.
//!
//! The desktop app owns the review command. CodexPlus only locates the active
//! composer, refuses to overwrite a draft, types `/review`, then follows the
//! same two Enter-key selections as the native UI.

#![cfg(windows)]

use std::time::{Duration, Instant};

use windows::core::{BSTR, VARIANT};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationValuePattern,
    TreeScope_Descendants, UIA_ButtonControlTypeId, UIA_ControlTypePropertyId,
    UIA_EditControlTypeId, UIA_NamePropertyId, UIA_ValuePatternId,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::FindWindowExW;

use super::native;

const RENDER_HOST_CLASS: &str = "Chrome_RenderWidgetHostHWND";
const COMPOSER_CLASS_PREFIX: &str = "ProseMirror";
const REVIEW_COMMAND: &str = "/review";
const PLAN_COMMAND: &str = "/plan";
const REVIEW_TARGET_TIMEOUT: Duration = Duration::from_secs(3);
const COMMAND_SETTLE_DELAY: Duration = Duration::from_millis(150);
const REVIEW_BUTTON_WIDTH: i32 = 28;
const REVIEW_BUTTON_GAP: i32 = 8;
const MODEL_CONTROL_RESERVED_WIDTH: i32 = 32;

const PERMISSION_NAMES: &[&str] = &["更改权限", "Change permissions"];
const STOP_NAMES: &[&str] = &["停止", "Stop"];
const UNCOMMITTED_REVIEW_NAMES: &[&str] = &["审查未提交的更改", "Review uncommitted changes"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionButtonState {
    pub prompt_rect: RECT,
    pub plan_rect: RECT,
    pub review_rect: RECT,
    pub enabled: bool,
}

struct PendingReview {
    target: HWND,
    expires_at: Instant,
    next_action_at: Instant,
    command: String,
    select_target: bool,
    stage: PendingStage,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PendingStage {
    SubmitCommand,
    SelectTarget,
}

pub struct ReviewAutomation {
    automation: Option<IUIAutomation>,
    pending: Option<PendingReview>,
}

impl Default for ReviewAutomation {
    fn default() -> Self {
        Self::new()
    }
}

impl ReviewAutomation {
    pub fn new() -> Self {
        unsafe {
            // S_FALSE means COM was already initialised on this thread. A
            // changed apartment is also non-fatal: CoCreateInstance can still
            // succeed against the thread's existing apartment.
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let automation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok();
            Self {
                automation,
                pending: None,
            }
        }
    }

    pub fn inspect_buttons(&self, target: HWND) -> Option<ActionButtonState> {
        let automation = self.automation.as_ref()?;
        let root = render_root(automation, target)?;
        let composer = find_composer(automation, &root)?;
        let composer_rect = unsafe { composer.CurrentBoundingRectangle().ok()? };
        let permission = find_named_element(automation, &root, PERMISSION_NAMES)?;
        let permission_rect = unsafe { permission.CurrentBoundingRectangle().ok()? };
        let row_top = permission_rect.top;
        let row_bottom = permission_rect.bottom;
        let model_rect = find_model_selector(automation, &root, composer_rect, permission_rect)?;
        let (prompt_rect, plan_rect, review_rect) =
            action_button_rects(model_rect.left, row_top, row_bottom, composer_rect.left)?;

        let enabled = composer_is_empty(&composer)
            && find_named_element(automation, &root, STOP_NAMES).is_none();

        Some(ActionButtonState {
            prompt_rect,
            plan_rect,
            review_rect,
            enabled,
        })
    }

    /// Submit `/review` only when the composer is empty and Codex is idle.
    pub fn begin_uncommitted_review(&mut self, target: HWND) -> bool {
        self.begin_command(target, REVIEW_COMMAND, true)
    }

    /// Submit `/plan` only when the composer is empty and Codex is idle.
    pub fn begin_plan(&mut self, target: HWND) -> bool {
        self.begin_command(target, PLAN_COMMAND, false)
    }

    /// Insert an ordinary user-authored prompt without activating slash-command
    /// semantics, then submit it with a real Enter key.
    pub fn begin_prompt(&mut self, target: HWND, prompt: &str) -> bool {
        let prompt = prompt.trim();
        if prompt.is_empty() || prompt.contains(['\r', '\n']) {
            return false;
        }
        let Some((composer, value_pattern)) = self.action_composer(target) else {
            return false;
        };
        if unsafe { value_pattern.SetValue(&BSTR::from(prompt)) }.is_err()
            || unsafe { composer.SetFocus() }.is_err()
        {
            return false;
        }
        self.set_pending(target, prompt, false);
        true
    }

    fn begin_command(&mut self, target: HWND, command: &str, select_target: bool) -> bool {
        let Some((composer, _value_pattern)) = self.action_composer(target) else {
            return false;
        };
        // ProseMirror must receive real keyboard events so Codex can recognize
        // the slash command and switch modes instead of sending literal text.
        if unsafe { composer.SetFocus() }.is_err() || !send_unicode_text(command) {
            return false;
        }
        self.set_pending(target, command, select_target);
        true
    }

    fn action_composer(
        &self,
        target: HWND,
    ) -> Option<(IUIAutomationElement, IUIAutomationValuePattern)> {
        let automation = self.automation.as_ref()?;
        let root = render_root(automation, target)?;
        let composer = find_composer(automation, &root)?;
        if !composer_is_empty(&composer) {
            return None;
        }
        if find_named_element(automation, &root, STOP_NAMES).is_some() {
            return None;
        }

        let Ok(value_pattern) = (unsafe {
            composer.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
        }) else {
            return None;
        };
        if unsafe {
            value_pattern
                .CurrentIsReadOnly()
                .map_or(true, |value| value.as_bool())
        } {
            return None;
        }
        Some((composer, value_pattern))
    }

    fn set_pending(&mut self, target: HWND, command: &str, select_target: bool) {
        let now = Instant::now();
        self.pending = Some(PendingReview {
            target,
            expires_at: now + REVIEW_TARGET_TIMEOUT,
            next_action_at: now + COMMAND_SETTLE_DELAY,
            command: command.to_owned(),
            select_target,
            stage: PendingStage::SubmitCommand,
        });
    }

    /// Continue the keyboard-driven slash-command flow.
    /// Returns true while the operation is still pending.
    pub fn advance(&mut self) -> bool {
        let Some(pending) = self.pending.as_ref() else {
            return false;
        };
        if Instant::now() >= pending.expires_at || !native::is_window_visible(pending.target) {
            if pending.stage == PendingStage::SubmitCommand {
                rollback_synthetic_command(
                    self.automation.as_ref(),
                    pending.target,
                    &pending.command,
                );
            }
            self.pending = None;
            return false;
        }
        if Instant::now() < pending.next_action_at {
            return true;
        }
        let Some(automation) = self.automation.as_ref() else {
            self.pending = None;
            return false;
        };
        let Some(root) = render_root(automation, pending.target) else {
            return true;
        };
        match pending.stage {
            PendingStage::SubmitCommand => {
                let submitted = native::root_owner(native::foreground_window())
                    == native::root_owner(pending.target)
                    && find_composer(automation, &root).is_some()
                    && send_enter_key();
                if submitted {
                    if pending.select_target {
                        if let Some(pending) = self.pending.as_mut() {
                            pending.stage = PendingStage::SelectTarget;
                            pending.next_action_at = Instant::now() + COMMAND_SETTLE_DELAY;
                        }
                    } else {
                        self.pending = None;
                        return false;
                    }
                }
            }
            PendingStage::SelectTarget => {
                let menu_ready =
                    named_action_near_composer_exists(automation, &root, UNCOMMITTED_REVIEW_NAMES);
                if menu_ready
                    && native::root_owner(native::foreground_window())
                        == native::root_owner(pending.target)
                    && send_enter_key()
                {
                    self.pending = None;
                    return false;
                }
            }
        }
        true
    }

    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
}

fn action_button_rects(
    model_left: i32,
    row_top: i32,
    row_bottom: i32,
    composer_left: i32,
) -> Option<(RECT, RECT, RECT)> {
    if row_bottom <= row_top {
        return None;
    }
    let review_right = model_left - MODEL_CONTROL_RESERVED_WIDTH;
    let review_left = review_right - REVIEW_BUTTON_WIDTH;
    let plan_right = review_left - REVIEW_BUTTON_GAP;
    let plan_left = plan_right - REVIEW_BUTTON_WIDTH;
    let prompt_right = plan_left - REVIEW_BUTTON_GAP;
    let prompt_left = prompt_right - REVIEW_BUTTON_WIDTH;
    if prompt_left < composer_left + REVIEW_BUTTON_GAP {
        return None;
    }
    Some((
        RECT {
            left: prompt_left,
            top: row_top,
            right: prompt_right,
            bottom: row_bottom,
        },
        RECT {
            left: plan_left,
            top: row_top,
            right: plan_right,
            bottom: row_bottom,
        },
        RECT {
            left: review_left,
            top: row_top,
            right: review_right,
            bottom: row_bottom,
        },
    ))
}

fn render_root(automation: &IUIAutomation, target: HWND) -> Option<IUIAutomationElement> {
    let class = native::wide_str(RENDER_HOST_CLASS);
    let renderer = unsafe {
        FindWindowExW(
            target,
            HWND::default(),
            windows::core::PCWSTR(class.as_ptr()),
            None,
        )
        .ok()?
    };
    unsafe { automation.ElementFromHandle(renderer).ok() }
}

fn find_composer(
    automation: &IUIAutomation,
    root: &IUIAutomationElement,
) -> Option<IUIAutomationElement> {
    let control_type = VARIANT::from(UIA_EditControlTypeId.0);
    let condition = unsafe {
        automation
            .CreatePropertyCondition(UIA_ControlTypePropertyId, &control_type)
            .ok()?
    };
    let elements = unsafe { root.FindAll(TreeScope_Descendants, &condition).ok()? };
    let count = unsafe { elements.Length().ok()? };
    let mut match_found = None;
    for index in 0..count {
        let Ok(element) = (unsafe { elements.GetElement(index) }) else {
            continue;
        };
        let class_name = unsafe { element.CurrentClassName().ok() }
            .map(|value| value.to_string())
            .unwrap_or_default();
        if class_name.starts_with(COMPOSER_CLASS_PREFIX) {
            if match_found.is_some() {
                return None;
            }
            match_found = Some(element);
        }
    }
    match_found
}

fn find_named_element(
    automation: &IUIAutomation,
    root: &IUIAutomationElement,
    names: &[&str],
) -> Option<IUIAutomationElement> {
    for name in names {
        let value = VARIANT::from(*name);
        let condition = unsafe {
            automation
                .CreatePropertyCondition(UIA_NamePropertyId, &value)
                .ok()?
        };
        if let Ok(element) = unsafe { root.FindFirst(TreeScope_Descendants, &condition) } {
            return Some(element);
        }
    }
    None
}

fn find_model_selector(
    automation: &IUIAutomation,
    root: &IUIAutomationElement,
    composer_rect: RECT,
    row_rect: RECT,
) -> Option<RECT> {
    let control_type = VARIANT::from(UIA_ButtonControlTypeId.0);
    let condition = unsafe {
        automation
            .CreatePropertyCondition(UIA_ControlTypePropertyId, &control_type)
            .ok()?
    };
    let elements = unsafe { root.FindAll(TreeScope_Descendants, &condition).ok()? };
    let count = unsafe { elements.Length().ok()? };
    let composer_center = (composer_rect.left + composer_rect.right) / 2;
    let mut best = None;
    for index in 0..count {
        let Ok(element) = (unsafe { elements.GetElement(index) }) else {
            continue;
        };
        let Ok(rect) = (unsafe { element.CurrentBoundingRectangle() }) else {
            continue;
        };
        let shares_row = rect.bottom > row_rect.top && rect.top < row_rect.bottom;
        let plausible_model_picker = shares_row
            && rect.left >= composer_center
            && rect.right <= composer_rect.right
            && rect.right - rect.left >= 48;
        if plausible_model_picker && best.is_none_or(|current: RECT| rect.left < current.left) {
            best = Some(rect);
        }
    }
    best
}

fn named_action_near_composer_exists(
    automation: &IUIAutomation,
    root: &IUIAutomationElement,
    names: &[&str],
) -> bool {
    let Some(composer) = find_composer(automation, root) else {
        return false;
    };
    let Ok(composer_rect) = (unsafe { composer.CurrentBoundingRectangle() }) else {
        return false;
    };
    for name in names {
        let value = VARIANT::from(*name);
        let Ok(condition) =
            (unsafe { automation.CreatePropertyCondition(UIA_NamePropertyId, &value) })
        else {
            continue;
        };
        let Ok(elements) = (unsafe { root.FindAll(TreeScope_Descendants, &condition) }) else {
            continue;
        };
        let Ok(count) = (unsafe { elements.Length() }) else {
            continue;
        };
        for index in 0..count {
            let Ok(element) = (unsafe { elements.GetElement(index) }) else {
                continue;
            };
            let Ok(rect) = (unsafe { element.CurrentBoundingRectangle() }) else {
                continue;
            };
            let near_composer = rect.bottom <= composer_rect.top
                && rect.bottom >= composer_rect.top - 500
                && rect.right > composer_rect.left
                && rect.left < composer_rect.right;
            if near_composer {
                return true;
            }
        }
    }
    false
}

fn composer_is_empty(composer: &IUIAutomationElement) -> bool {
    let Ok(pattern) =
        (unsafe { composer.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) })
    else {
        return false;
    };
    let value = unsafe { pattern.CurrentValue().ok() }
        .map(|value| value.to_string())
        .unwrap_or_default();
    let name = unsafe { composer.CurrentName().ok() }
        .map(|value| value.to_string())
        .unwrap_or_default();
    value_represents_empty_composer(&value, &name)
}

fn rollback_synthetic_command(automation: Option<&IUIAutomation>, target: HWND, command: &str) {
    let Some(automation) = automation else {
        return;
    };
    let Some(root) = render_root(automation, target) else {
        return;
    };
    let Some(composer) = find_composer(automation, &root) else {
        return;
    };
    let Ok(pattern) =
        (unsafe { composer.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) })
    else {
        return;
    };
    let value = unsafe { pattern.CurrentValue().ok() }
        .map(|value| value.to_string())
        .unwrap_or_default();
    if value.trim() == command {
        let _ = unsafe { pattern.SetValue(&BSTR::new()) };
    }
}

fn value_represents_empty_composer(value: &str, accessible_name: &str) -> bool {
    let value = value.trim();
    value.is_empty() || (!accessible_name.trim().is_empty() && value == accessible_name.trim())
}

fn send_enter_key() -> bool {
    let down = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_RETURN,
                ..Default::default()
            },
        },
    };
    let up = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_RETURN,
                dwFlags: KEYEVENTF_KEYUP,
                ..Default::default()
            },
        },
    };
    unsafe { SendInput(&[down, up], std::mem::size_of::<INPUT>() as i32) == 2 }
}

fn send_unicode_text(text: &str) -> bool {
    let units = text.encode_utf16().collect::<Vec<_>>();
    let mut inputs = Vec::with_capacity(units.len() * 2);
    for unit in units {
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wScan: unit,
                    dwFlags: KEYEVENTF_UNICODE,
                    ..Default::default()
                },
            },
        });
        inputs.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wScan: unit,
                    dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                    ..Default::default()
                },
            },
        });
    }
    !inputs.is_empty()
        && unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) } == inputs.len() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_or_placeholder_value_is_empty() {
        assert!(value_represents_empty_composer("", "随心输入"));
        assert!(value_represents_empty_composer("\n 随心输入 ", "随心输入"));
        assert!(value_represents_empty_composer(
            "Ask anything",
            "Ask anything"
        ));
    }

    #[test]
    fn user_draft_is_never_considered_empty() {
        assert!(!value_represents_empty_composer("保留我的草稿", "随心输入"));
        assert!(!value_represents_empty_composer("/review", "随心输入"));
    }

    #[test]
    fn button_state_remains_plain_data() {
        let state = ActionButtonState {
            prompt_rect: RECT {
                left: -26,
                top: 20,
                right: 2,
                bottom: 48,
            },
            plan_rect: RECT {
                left: 10,
                top: 20,
                right: 38,
                bottom: 48,
            },
            review_rect: RECT {
                left: 46,
                top: 20,
                right: 74,
                bottom: 48,
            },
            enabled: true,
        };
        assert_eq!(
            state.plan_rect.right - state.plan_rect.left,
            REVIEW_BUTTON_WIDTH
        );
        assert_eq!(
            state.review_rect.right - state.review_rect.left,
            REVIEW_BUTTON_WIDTH
        );
        assert!(state.plan_rect.right < state.review_rect.left);
        assert!(state.enabled);
    }

    #[test]
    fn buttons_are_anchored_immediately_left_of_model_picker() {
        let (prompt, plan, review) = action_button_rects(600, 20, 48, 10).unwrap();
        assert_eq!(review.right, 600 - MODEL_CONTROL_RESERVED_WIDTH);
        assert_eq!(review.right - review.left, REVIEW_BUTTON_WIDTH);
        assert_eq!(plan.right - plan.left, REVIEW_BUTTON_WIDTH);
        assert_eq!(review.left - plan.right, REVIEW_BUTTON_GAP);
        assert_eq!(plan.left - prompt.right, REVIEW_BUTTON_GAP);
    }

    #[test]
    fn buttons_hide_when_model_picker_has_insufficient_left_space() {
        assert!(action_button_rects(70, 20, 48, 10).is_none());
        assert!(action_button_rects(600, 48, 20, 10).is_none());
    }
}
