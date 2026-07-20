//! # Telegram HTML Italic Normalizer
//!
//! Normalizes markdown italic syntax (asterisks and underscores) to prevent unbalanced tags.
//! Standardizes them into HTML elements compatible with Teloxide output formatting rules.

fn is_whitespace_or_punct(c: char, delim: char) -> bool {
    c.is_whitespace() || (c.is_ascii_punctuation() && c != delim)
}

pub fn format_italic_delim(text: &str, delim: char, tag_open: &str, tag_close: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut open_idx = None;
    let mut pairs = std::collections::HashMap::new();

    let mut idx = 0;
    while idx < chars.len() {
        if chars[idx] == delim {
            let is_left = (idx == 0 || is_whitespace_or_punct(chars[idx - 1], delim))
                && (idx + 1 < chars.len() && !chars[idx + 1].is_whitespace() && chars[idx + 1] != delim);
            let is_right = (idx > 0 && !chars[idx - 1].is_whitespace() && chars[idx - 1] != delim)
                && (idx + 1 == chars.len() || is_whitespace_or_punct(chars[idx + 1], delim));

            if is_left {
                open_idx = Some(idx);
            } else if is_right {
                if let Some(left) = open_idx {
                    pairs.insert(left, idx);
                    open_idx = None;
                }
            }
        }
        idx += 1;
    }

    let mut result = String::new();
    let mut idx = 0;
    while idx < chars.len() {
        if chars[idx] == delim {
            if pairs.contains_key(&idx) {
                result.push_str(tag_open);
            } else if pairs.values().any(|&right| right == idx) {
                result.push_str(tag_close);
            } else {
                result.push(chars[idx]);
            }
        } else {
            result.push(chars[idx]);
        }
        idx += 1;
    }
    result
}
