//! # Telegram HTML Formatting and Message Splitting
//!
//! This module converts Markdown formatting to Telegram-compatible HTML tags
//! and splits long messages safely without breaking HTML tag pairs.

use regex::{Regex, Captures};

const SENTINEL: char = '\x00';

fn placeholder(kind: &str, idx: usize) -> String {
    format!("{}{}{}{}", SENTINEL, kind, idx, SENTINEL)
}

/// Convert basic Markdown syntax to Telegram HTML.
/// Handles: code blocks, inline code, bold, italic, strikethrough, spoiler, and links.
/// Convert basic Markdown syntax to Telegram HTML.
/// Handles: code blocks, inline code, bold, italic, strikethrough, spoiler, and links.
pub fn markdown_to_telegram_html(text: &str) -> String {
    let mut code_blocks: Vec<(String, String)> = Vec::new();
    let mut inline_codes: Vec<String> = Vec::new();
    let mut links: Vec<(String, String)> = Vec::new();

    let text_buf = extract_entities(text, &mut code_blocks, &mut inline_codes, &mut links);
    let escaped = html_escape::encode_safe(&text_buf).to_string();
    let formatted = apply_markdown_formatting(&escaped);

    restore_entities(formatted, &code_blocks, &inline_codes, &links)
}

fn extract_entities(
    text: &str,
    code_blocks: &mut Vec<(String, String)>,
    inline_codes: &mut Vec<String>,
    links: &mut Vec<(String, String)>,
) -> String {
    // 1. Extract code blocks
    let cb_re = Regex::new(r"(?s)```(\w*)\n(.*?)```").unwrap();
    let mut text_buf = cb_re.replace_all(text, |caps: &Captures| {
        let lang = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let code = caps.get(2).map_or("", |m| m.as_str()).to_string();
        let idx = code_blocks.len();
        code_blocks.push((lang, code));
        placeholder("CB", idx)
    }).to_string();

    // 2. Extract inline code
    let ic_re = Regex::new(r"`([^`\n]+)`").unwrap();
    text_buf = ic_re.replace_all(&text_buf, |caps: &Captures| {
        let code = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let idx = inline_codes.len();
        inline_codes.push(code);
        placeholder("IC", idx)
    }).to_string();

    // 3. Extract links
    let link_re = Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap();
    link_re.replace_all(&text_buf, |caps: &Captures| {
        let link_text = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let link_url = caps.get(2).map_or("", |m| m.as_str()).to_string();
        let idx = links.len();
        links.push((link_text, link_url));
        placeholder("LK", idx)
    }).to_string()
}

fn restore_entities(
    mut text: String,
    code_blocks: &[(String, String)],
    inline_codes: &[String],
    links: &[(String, String)],
) -> String {
    // 1. Restore inline codes
    for (i, code) in inline_codes.iter().enumerate() {
        let p = placeholder("IC", i);
        let escaped_code = html_escape::encode_safe(code).to_string();
        text = text.replace(&p, &format!("<code>{}</code>", escaped_code));
    }

    // 2. Restore code blocks
    for (i, (lang, code)) in code_blocks.iter().enumerate() {
        let p = placeholder("CB", i);
        let escaped_code = html_escape::encode_safe(code).to_string();
        let block = if !lang.is_empty() {
            format!("<pre><code class=\"language-{}\">{}</code></pre>", html_escape::encode_safe(lang), escaped_code)
        } else {
            format!("<pre>{}</pre>", escaped_code)
        };
        text = text.replace(&p, &block);
    }

    // 3. Restore links
    for (i, (l_text, l_url)) in links.iter().enumerate() {
        let p = placeholder("LK", i);
        let escaped_text = html_escape::encode_safe(l_text).to_string();
        let escaped_url = html_escape::encode_double_quoted_attribute(l_url).to_string();
        text = text.replace(&p, &format!("<a href=\"{}\">{}</a>", escaped_url, escaped_text));
    }

    text
}

fn apply_markdown_formatting(escaped: &str) -> String {
    let h_re = Regex::new(r"(?m)^#{1,6}\s+(.+)$").unwrap();
    let mut out = h_re.replace_all(escaped, "<b>$1</b>").to_string();

    let bold_re = Regex::new(r"(?s)\*\*(.+?)\*\*").unwrap();
    out = bold_re.replace_all(&out, "<b>$1</b>").to_string();

    let italic_re = Regex::new(r"\*(.+?)\*").unwrap();
    out = italic_re.replace_all(&out, "<i>$1</i>").to_string();

    let strike_re = Regex::new(r"~~(.+?)~~").unwrap();
    out = strike_re.replace_all(&out, "<s>$1</s>").to_string();

    let spoiler_re = Regex::new(r"\|\|(.+?)\|\|").unwrap();
    out = spoiler_re.replace_all(&out, "<tg-spoiler>$1</tg-spoiler>").to_string();
    out
}

fn get_tag_name(tag: &str) -> &str {
    let stripped = tag.trim_matches(|c| c == '<' || c == '>');
    stripped.split_whitespace().next().unwrap_or("").trim_start_matches('/')
}

/// Split HTML message into chunks that fit max_len without breaking tag pairs.
pub fn split_html_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let tokens = tokenize_html(text);
    let mut chunks = Vec::new();
    let mut current_parts: Vec<String> = Vec::new();
    let mut current_len = 0;
    let mut open_tags: Vec<String> = Vec::new();

    for token in tokens {
        if token.is_empty() {
            continue;
        }

        let is_tag = token.starts_with('<') && token.ends_with('>');
        let closing_tags_str = get_closing_tags(&open_tags);

        // If this token exceeds the remaining space in the chunk, wrap up current chunk
        if current_parts.iter().any(|s| !s.is_empty()) && (current_len + token.len() + closing_tags_str.len() > max_len) {
            current_parts.push(closing_tags_str);
            chunks.push(current_parts.join(""));
            
            // Start new chunk and re-open active tags
            current_parts = open_tags.clone();
            current_len = open_tags.iter().map(|s| s.len()).sum();
        }

        current_parts.push(token.to_string());
        current_len += token.len();

        if is_tag {
            if token.starts_with("</") {
                let name = get_tag_name(token);
                if let Some(pos) = open_tags.iter().rposition(|t| get_tag_name(t) == name) {
                    open_tags.remove(pos);
                }
            } else if !token.ends_with("/>") {
                open_tags.push(token.to_string());
            }
        }
    }

    if !current_parts.is_empty() {
        let closing_tags_str = get_closing_tags(&open_tags);
        current_parts.push(closing_tags_str);
        chunks.push(current_parts.join(""));
    }

    chunks.into_iter().filter(|c| !c.trim().is_empty()).collect()
}

fn get_closing_tags(open_tags: &[String]) -> String {
    open_tags.iter().rev()
        .map(|t| format!("</{}>", get_tag_name(t)))
        .collect::<Vec<_>>()
        .join("")
}

fn tokenize_html(text: &str) -> Vec<&str> {
    let tok_re = Regex::new(r"(</?[a-zA-Z][^>]*>|\n\n|\n)").unwrap();
    let mut tokens = Vec::new();
    let mut last_idx = 0;

    for mat in tok_re.find_iter(text) {
        if mat.start() > last_idx {
            tokens.push(&text[last_idx..mat.start()]);
        }
        tokens.push(mat.as_str());
        last_idx = mat.end();
    }
    if last_idx < text.len() {
        tokens.push(&text[last_idx..]);
    }
    tokens
}
