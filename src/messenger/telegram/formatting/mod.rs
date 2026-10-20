//! # Telegram HTML Formatting and Message Splitting
//!
//! ## Overview
//! Converts Markdown formatting to Telegram-compatible HTML tags and splits long messages
//! safely across chunk boundaries without breaking HTML tag pairs.
//!
//! ## Submodules
//! - [`helpers`] (`helpers.rs`): Converts Markdown syntax (links, code blocks, bold) to HTML.
//! - [`splitting`] (`splitting.rs`): Truncates and chunks HTML payloads to comply with Telegram length limits.
//! - [`table`] (`table.rs`): Formats Markdown pipe-tables into aligned monospace blocks.
//!
//! ## Search Tags
//! #formatting-index, #markdown-to-html, #message-splitting, #ui-formatting

use regex::{Captures, Regex};

pub mod helpers;
pub mod splitting;
pub mod table;

pub use helpers::markdown_to_telegram_html;
pub use splitting::split_html_message;

/// Executes `find_best_option` (find best option).
pub fn find_best_option(text: &str, options: &[String]) -> Option<usize> {
    let t = text.to_lowercase();
    let words: std::collections::HashSet<_> = t
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    if words.is_empty() {
        return None;
    }
    let mut best = None;
    let mut max_score = 0;
    for (i, opt) in options.iter().enumerate() {
        let o = opt.to_lowercase();
        if o.contains(&t) || t.contains(&o) {
            return Some(i);
        }
        let o_words: std::collections::HashSet<_> = o
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .collect();
        let score = words.intersection(&o_words).count();
        if score > max_score {
            max_score = score;
            best = Some(i);
        }
    }
    (max_score > 0).then_some(best).flatten()
}

/// Executes `convert_blockquotes` (convert blockquotes).
pub fn convert_blockquotes(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut result = Vec::new();
    let mut quote_buf = Vec::new();
    let escaped_gt = "&gt; ";
    let escaped_gt_exp = "&gt;! ";
    let mut is_expandable = false;

    for line in lines {
        if let Some(rest) = line.strip_prefix(escaped_gt_exp) {
            is_expandable = true;
            quote_buf.push(rest);
        } else if let Some(rest) = line.strip_prefix(escaped_gt) {
            quote_buf.push(rest);
        } else {
            if !quote_buf.is_empty() {
                let tag = if is_expandable {
                    "<blockquote expandable>"
                } else {
                    "<blockquote>"
                };
                result.push(format!("{}{}{}</blockquote>", tag, quote_buf.join("\n"), ""));
                quote_buf.clear();
                is_expandable = false;
            }
            result.push(line.to_string());
        }
    }

    if !quote_buf.is_empty() {
        let tag = if is_expandable {
            "<blockquote expandable>"
        } else {
            "<blockquote>"
        };
        result.push(format!("{}{}{}</blockquote>", tag, quote_buf.join("\n"), ""));
    }

    result.join("\n")
}

fn mask_code(text: &str, saved: &mut Vec<String>) -> String {
    let mut masked = if let Ok(cb_re) = Regex::new(r"(?s)```.*?```") {
        cb_re
            .replace_all(text, |caps: &Captures| {
                let idx = saved.len();
                saved.push(caps[0].to_string());
                helpers::placeholder("CODE", idx)
            })
            .to_string()
    } else {
        text.to_string()
    };

    if let Ok(ic_re) = Regex::new(r"`[^`\n]+`") {
        masked = ic_re
            .replace_all(&masked, |caps: &Captures| {
                let idx = saved.len();
                saved.push(caps[0].to_string());
                helpers::placeholder("CODE", idx)
            })
            .to_string();
    }

    masked
}

fn restore_code(mut text: String, saved: &[String]) -> String {
    for (i, original) in saved.iter().enumerate() {
        let p = helpers::placeholder("CODE", i);
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

fn is_whitespace_or_punct(c: char, delim: char) -> bool {
    c.is_whitespace() || (c.is_ascii_punctuation() && c != delim)
}

fn collect_italic_pairs(chars: &[char], delim: char) -> std::collections::HashMap<usize, usize> {
    let (mut open_idx, mut pairs, mut in_html_tag) = (None, std::collections::HashMap::new(), false);
    for (idx, &ch) in chars.iter().enumerate() {
        if ch == '<' {
            in_html_tag = true;
            open_idx = None;
            continue;
        }
        if ch == '>' || ch == '\n' {
            if ch == '>' {
                in_html_tag = false;
            }
            open_idx = None;
            continue;
        }
        if !in_html_tag && ch == delim {
            let is_left = (idx == 0 || is_whitespace_or_punct(chars[idx - 1], delim))
                && (idx + 1 < chars.len() && !chars[idx + 1].is_whitespace() && chars[idx + 1] != delim);
            let is_right = (idx > 0 && !chars[idx - 1].is_whitespace() && chars[idx - 1] != delim)
                && (idx + 1 == chars.len() || is_whitespace_or_punct(chars[idx + 1], delim));
            if is_right && let Some(left) = open_idx.take() {
                pairs.insert(left, idx);
            } else if is_left && !is_right {
                open_idx = Some(idx);
            }
        }
    }
    pairs
}

/// Executes `format_italic_delim` (format italic delim).
pub fn format_italic_delim(text: &str, delim: char, tag_open: &str, tag_close: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let pairs = collect_italic_pairs(&chars, delim);
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

/// Blockquote conversion re-exports.
pub mod blockquotes {
    pub use super::convert_blockquotes;
}

/// Inline callback button stripping re-exports.
pub mod buttons {
    pub use super::strip_button_syntax;
}

/// Markdown italic delimiter normalization re-exports.
pub mod italic {
    pub use super::format_italic_delim;
}
