//! # Telegram HTML Italic Normalizer
//!
//! ## Overview
//! Normalizes markdown italic syntax (asterisks and underscores) to prevent unbalanced tags.
//! Standardizes them into HTML elements compatible with Teloxide output formatting rules.
//!
//! ## Search Tags
//! #italic

fn is_whitespace_or_punct(c: char, delim: char) -> bool {
    c.is_whitespace() || (c.is_ascii_punctuation() && c != delim)
}

/// Executes `format_italic_delim` (format italic delim).
pub fn format_italic_delim(text: &str, delim: char, tag_open: &str, tag_close: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut open_idx = None;
    let mut pairs = std::collections::HashMap::new();
    let mut in_html_tag = false;

    let mut idx = 0;
    while idx < chars.len() {
        let ch = chars[idx];
        if ch == '<' {
            in_html_tag = true;
            open_idx = None;
            idx += 1;
            continue;
        }
        if ch == '>' {
            in_html_tag = false;
            open_idx = None;
            idx += 1;
            continue;
        }
        if ch == '\n' {
            open_idx = None;
            idx += 1;
            continue;
        }
        if !in_html_tag && ch == delim {
            let is_left = (idx == 0 || is_whitespace_or_punct(chars[idx - 1], delim))
                && (idx + 1 < chars.len() && !chars[idx + 1].is_whitespace() && chars[idx + 1] != delim);
            let is_right = (idx > 0 && !chars[idx - 1].is_whitespace() && chars[idx - 1] != delim)
                && (idx + 1 == chars.len() || is_whitespace_or_punct(chars[idx + 1], delim));

            if is_right && open_idx.is_some() {
                if let Some(left) = open_idx.take() {
                    pairs.insert(left, idx);
                }
            } else if is_left && !is_right {
                open_idx = Some(idx);
            }
        }
        idx += 1;
    }

    let mut result = String::new();
    for (idx, &ch) in chars.iter().enumerate() {
        if ch == delim {
            if pairs.contains_key(&idx) {
                result.push_str(tag_open);
            } else if pairs.values().any(|&right| right == idx) {
                result.push_str(tag_close);
            } else {
                result.push(ch);
            }
        } else {
            result.push(ch);
        }
    }
    result
}
