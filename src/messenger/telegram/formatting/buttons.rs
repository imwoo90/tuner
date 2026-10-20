//! # Inline Callback Buttons Filter
//!
//! ## Overview
//! Parses and strips inline dynamic buttons (`[button:Label]`) embedded by agents in their replies.
//! Prevents exposing raw callback payload structures to standard messaging clients.
//!
//! ## Search Tags
//! #buttons

use regex::{Regex, Captures};

fn mask_code(text: &str, saved: &mut Vec<String>) -> String {
    let mut masked = if let Ok(cb_re) = Regex::new(r"(?s)```.*?```") {
        cb_re.replace_all(text, |caps: &Captures| {
            let idx = saved.len();
            saved.push(caps[0].to_string());
            super::helpers::placeholder("CODE", idx)
        }).to_string()
    } else {
        text.to_string()
    };

    if let Ok(ic_re) = Regex::new(r"`[^`\n]+`") {
        masked = ic_re.replace_all(&masked, |caps: &Captures| {
            let idx = saved.len();
            saved.push(caps[0].to_string());
            super::helpers::placeholder("CODE", idx)
        }).to_string();
    }

    masked
}

fn restore_code(mut text: String, saved: &[String]) -> String {
    for (i, original) in saved.iter().enumerate() {
        let p = super::helpers::placeholder("CODE", i);
        text = text.replace(&p, original);
    }
    text
}

fn collapse_blank_lines(text: &str) -> String {
    if let Ok(re) = Regex::new(r"\n{3,}") {
        re.replace_all(text, "\n\n").to_string()
    } else {
        text.to_string()
    }
}

/// Executes `strip_button_syntax` (strip button syntax).
pub fn strip_button_syntax(text: &str) -> String {
    if text.is_empty() || !text.contains("[button:") {
        return text.to_string();
    }

    let mut saved = Vec::new();
    let masked = mask_code(text, &mut saved);

    let stripped = if let Ok(btn_re) = Regex::new(r"\[button:[^\]]+\]") {
        btn_re.replace_all(&masked, "").to_string()
    } else {
        masked
    };

    let restored = restore_code(stripped, &saved);
    collapse_blank_lines(&restored).trim().to_string()
}
