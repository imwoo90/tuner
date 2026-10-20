//! # Telegram Formatting Common Helpers
//!
//! ## Overview
//! Provides shared utilities and regex logic to parse placeholders, process code blocks,
//! handle specific HTML entity encodings, and strip interactive UI features before delivery.
//!
//! ## Search Tags
//! #helpers

use regex::{Regex, Captures};

const SENTINEL: char = '\x00';

/// Executes `placeholder` (placeholder).
pub fn placeholder(kind: &str, idx: usize) -> String {
    format!("{}{}{}{}", SENTINEL, kind, idx, SENTINEL)
}

/// Executes `markdown_to_telegram_html` (markdown to telegram html).
pub fn markdown_to_telegram_html(text: &str) -> String {
    let cleaned_text = super::buttons::strip_button_syntax(text);

    let mut code_blocks: Vec<(String, String, bool)> = Vec::new();
    let mut inline_codes: Vec<String> = Vec::new();
    let mut links: Vec<(String, String)> = Vec::new();
    let mut table_blocks: Vec<String> = Vec::new();

    let text_buf = extract_entities(&cleaned_text, &mut code_blocks, &mut inline_codes, &mut links, &mut table_blocks);
    let escaped = html_escape::encode_safe(&text_buf).to_string();
    let formatted = apply_markdown_formatting(&escaped);

    restore_entities(formatted, &code_blocks, &inline_codes, &links, &table_blocks)
}

fn sanitize_code_block(code: &str, is_quoted: bool) -> String {
    let raw_lines: Vec<&str> = code.split('\n').collect();
    if raw_lines.is_empty() {
        return code.to_string();
    }

    let has_quote_prefix = raw_lines.iter().any(|l| {
        let t = l.trim_start();
        t.starts_with(">! ") || t.starts_with(">!") || (is_quoted && (t.starts_with("> ") || t.starts_with(">")))
    }) && raw_lines.iter().all(|l| {
        let t = l.trim_start();
        t.is_empty()
            || t.starts_with(">! ")
            || t.starts_with(">!")
            || (is_quoted && (t.starts_with("> ") || t.starts_with(">")))
    });

    if !has_quote_prefix {
        return code.to_string();
    }

    let mut cleaned_lines = Vec::new();
    let total = raw_lines.len();
    for (i, line) in raw_lines.iter().enumerate() {
        let trimmed = line.trim();
        if i == total - 1 && (trimmed == ">!" || trimmed == ">" || trimmed.is_empty()) {
            continue;
        }

        if let Some(rest) = line.strip_prefix(">! ") {
            cleaned_lines.push(rest);
        } else if let Some(rest) = line.strip_prefix("> ") {
            cleaned_lines.push(rest);
        } else if let Some(rest) = line.strip_prefix(">!") {
            cleaned_lines.push(rest);
        } else if let Some(rest) = line.strip_prefix(">") {
            cleaned_lines.push(rest);
        } else if trimmed == ">!" || trimmed == ">" {
            cleaned_lines.push("");
        } else {
            cleaned_lines.push(*line);
        }
    }

    let mut result = cleaned_lines.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

fn extract_entities(
    text: &str,
    code_blocks: &mut Vec<(String, String, bool)>,
    inline_codes: &mut Vec<String>,
    links: &mut Vec<(String, String)>,
    table_blocks: &mut Vec<String>,
) -> String {
    let mut text_buf = if let Ok(cb_re) = Regex::new(r"(?s)([ \t]*>[!]?[ \t]*)?```(\w*)\n(.*?)```") {
        cb_re.replace_all(text, |caps: &Captures| {
            let prefix_quote = caps.get(1).map_or("", |m| m.as_str());
            let lang = caps.get(2).map_or("", |m| m.as_str()).to_string();
            let code = caps.get(3).map_or("", |m| m.as_str());
            let is_quoted = !prefix_quote.trim().is_empty();
            let sanitized_code = sanitize_code_block(code, is_quoted);
            let idx = code_blocks.len();
            code_blocks.push((lang, sanitized_code, is_quoted));
            format!("{}{}", prefix_quote, placeholder("CB", idx))
        }).to_string()
    } else {
        text.to_string()
    };

    text_buf = super::table::extract_tables(&text_buf, table_blocks);

    if let Ok(ic_re) = Regex::new(r"`([^`\n]+)`") {
        text_buf = ic_re.replace_all(&text_buf, |caps: &Captures| {
            let code = caps.get(1).map_or("", |m| m.as_str()).to_string();
            let idx = inline_codes.len();
            inline_codes.push(code);
            placeholder("IC", idx)
        }).to_string();
    }

    if let Ok(link_re) = Regex::new(r"\[([^\]]+)\]\(([^)]+)\)") {
        link_re.replace_all(&text_buf, |caps: &Captures| {
            let link_text = caps.get(1).map_or("", |m| m.as_str()).to_string();
            let link_url = caps.get(2).map_or("", |m| m.as_str()).to_string();
            let idx = links.len();
            links.push((link_text, link_url));
            placeholder("LK", idx)
        }).to_string()
    } else {
        text_buf
    }
}

fn restore_entities(
    mut text: String,
    code_blocks: &[(String, String, bool)],
    inline_codes: &[String],
    links: &[(String, String)],
    table_blocks: &[String],
) -> String {
    for (i, table_text) in table_blocks.iter().enumerate() {
        let p = placeholder("TB", i);
        let escaped_table = html_escape::encode_safe(table_text).to_string();
        text = text.replace(&p, &format!("<pre>{}</pre>", escaped_table));
    }

    for (i, (l_text, l_url)) in links.iter().enumerate() {
        let p = placeholder("LK", i);
        let escaped_text = html_escape::encode_safe(l_text).to_string();
        let formatted_text = apply_markdown_formatting(&escaped_text);
        let escaped_url = html_escape::encode_double_quoted_attribute(l_url).to_string();
        text = text.replace(&p, &format!("<a href=\"{}\">{}</a>", escaped_url, formatted_text));
    }

    for (i, code) in inline_codes.iter().enumerate() {
        let p = placeholder("IC", i);
        let escaped_code = html_escape::encode_safe(code).to_string();
        text = text.replace(&p, &format!("<code>{}</code>", escaped_code));
    }

    for (i, (lang, code, is_quoted)) in code_blocks.iter().enumerate() {
        let p = placeholder("CB", i);
        let escaped_code = html_escape::encode_safe(code).to_string();
        let raw_block = if !lang.is_empty() {
            format!("<pre><code class=\"language-{}\">{}</code></pre>", html_escape::encode_safe(lang), escaped_code)
        } else {
            format!("<pre>{}</pre>", escaped_code)
        };
        let block = if *is_quoted {
            raw_block
        } else {
            format!("<blockquote expandable>{}</blockquote>", raw_block)
        };
        text = text.replace(&p, &block);
    }

    text
}

/// Executes `apply_markdown_formatting` (apply markdown formatting).
pub fn apply_markdown_formatting(escaped: &str) -> String {
    let mut out = escaped.to_string();
    let rules = [
        (r"(?m)^#{1,6}\s+(.+)$", "<b>$1</b>"),
        (r"\*\*\*([^\*\n]+?)\*\*\*", "<b><i>$1</i></b>"),
        (r"(?s)\*\*(.+?\*?)\*\*", "<b>$1</b>"),
    ];
    for (pat, rep) in rules {
        if let Ok(re) = Regex::new(pat) {
            out = re.replace_all(&out, rep).to_string();
        }
    }

    out = super::italic::format_italic_delim(&out, '*', "<i>", "</i>");
    out = super::italic::format_italic_delim(&out, '_', "<i>", "</i>");

    for (pat, rep) in [
        (r"~~(.+?)~~", "<s>$1</s>"),
        (r"\|\|(.+?)\|\|", "<tg-spoiler>$1</tg-spoiler>"),
    ] {
        if let Ok(re) = Regex::new(pat) {
            out = re.replace_all(&out, rep).to_string();
        }
    }

    out = super::blockquotes::convert_blockquotes(&out);

    for (pat, rep) in [
        (r"(?m)^[-*]{3,}$", "———"),
        (r"(?m)^[-*]\s+", "• "),
    ] {
        if let Ok(re) = Regex::new(pat) {
            out = re.replace_all(&out, rep).to_string();
        }
    }

    out
}
