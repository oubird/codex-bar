//! Minimal tray-icon helper. Lifecycle is owned by [`run_app`] in
//! [`crate::ui::native`], so this module only contributes the toast text
//! formatter and a stand-alone struct used for tests.

#![cfg(windows)]

#[derive(Debug)]
pub struct TrayRequest {
    pub text: String,
}

pub fn build_request_text(state_label: &str, tooltip: &str) -> TrayRequest {
    let mut text = String::new();
    text.push_str(state_label);
    if !tooltip.is_empty() {
        text.push('\n');
        text.push_str(tooltip);
    }
    TrayRequest { text }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_text_combines_state_and_tooltip() {
        let req = build_request_text("OK", "5h 42%, reset in 1h18m");
        assert_eq!(req.text, "OK\n5h 42%, reset in 1h18m");
    }
}
