//! # Inline Callback Buttons Filter
//!
//! Parses and strips inline dynamic buttons (`[button:Label]`) embedded by agents in their replies.
//! Prevents exposing raw callback payload structures to standard messaging clients.

use regex::{Regex, Captures};

fn mask_code(text: &str, saved: &mut Vec<String>) -> String {
    let cb_re = Regex::new(r"(?s)```.*?```").unwrap();
    let mut masked = cb_re.replace_all(text, |caps: &Captures| {
        let idx = saved.len();
        saved.push(caps.get(0).unwrap().as_str().to_string());
        super::helpers::placeholder("CODE", idx)
    }).to_string();

    let ic_re = Regex::new(r"`[^`\n]+`").unwrap();
    masked = ic_re.replace_all(&masked, |caps: &Captures| {
        let idx = saved.len();
        saved.push(caps.get(0).unwrap().as_str().to_string());
        super::helpers::placeholder("CODE", idx)
    }).to_string();

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
    let re = Regex::new(r"\n{3,}").unwrap();
    re.replace_all(text, "\n\n").to_string()
}

pub fn strip_button_syntax(text: &str) -> String {
    if text.is_empty() || !text.contains("[button:") {
        return text.to_string();
    }

    let mut saved = Vec::new();
    let masked = mask_code(text, &mut saved);
    
    let btn_re = Regex::new(r"\[button:[^\]]+\]").unwrap();
    let stripped = btn_re.replace_all(&masked, "").to_string();
    
    let restored = restore_code(stripped, &saved);
    collapse_blank_lines(&restored).trim().to_string()
}
