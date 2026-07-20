//! # Telegram Formatting Common Helpers
//!
//! Provides shared utilities and regex logic to parse placeholders, process code blocks,
//! handle specific HTML entity encodings, and strip interactive UI features before delivery.

use regex::{Regex, Captures};

const SENTINEL: char = '\x00';

pub fn placeholder(kind: &str, idx: usize) -> String {
    format!("{}{}{}{}", SENTINEL, kind, idx, SENTINEL)
}

pub fn markdown_to_telegram_html(text: &str) -> String {
    let cleaned_text = super::buttons::strip_button_syntax(text);

    let mut code_blocks: Vec<(String, String)> = Vec::new();
    let mut inline_codes: Vec<String> = Vec::new();
    let mut links: Vec<(String, String)> = Vec::new();
    let mut table_blocks: Vec<String> = Vec::new();

    let text_buf = extract_entities(&cleaned_text, &mut code_blocks, &mut inline_codes, &mut links, &mut table_blocks);
    let escaped = html_escape::encode_safe(&text_buf).to_string();
    let formatted = apply_markdown_formatting(&escaped);

    restore_entities(formatted, &code_blocks, &inline_codes, &links, &table_blocks)
}

fn extract_entities(
    text: &str,
    code_blocks: &mut Vec<(String, String)>,
    inline_codes: &mut Vec<String>,
    links: &mut Vec<(String, String)>,
    table_blocks: &mut Vec<String>,
) -> String {
    let cb_re = Regex::new(r"(?s)```(\w*)\n(.*?)```").unwrap();
    let mut text_buf = cb_re.replace_all(text, |caps: &Captures| {
        let lang = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let code = caps.get(2).map_or("", |m| m.as_str()).to_string();
        let idx = code_blocks.len();
        code_blocks.push((lang, code));
        placeholder("CB", idx)
    }).to_string();

    text_buf = super::table::extract_tables(&text_buf, table_blocks);

    let ic_re = Regex::new(r"`([^`\n]+)`").unwrap();
    text_buf = ic_re.replace_all(&text_buf, |caps: &Captures| {
        let code = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let idx = inline_codes.len();
        inline_codes.push(code);
        placeholder("IC", idx)
    }).to_string();

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

    text
}

pub fn apply_markdown_formatting(escaped: &str) -> String {
    let h_re = Regex::new(r"(?m)^#{1,6}\s+(.+)$").unwrap();
    let mut out = h_re.replace_all(escaped, "<b>$1</b>").to_string();

    let bold_re = Regex::new(r"(?s)\*\*(.+?)\*\*").unwrap();
    out = bold_re.replace_all(&out, "<b>$1</b>").to_string();

    out = super::italic::format_italic_delim(&out, '*', "<i>", "</i>");
    out = super::italic::format_italic_delim(&out, '_', "<i>", "</i>");

    let strike_re = Regex::new(r"~~(.+?)~~").unwrap();
    out = strike_re.replace_all(&out, "<s>$1</s>").to_string();

    let spoiler_re = Regex::new(r"\|\|(.+?)\|\|").unwrap();
    out = spoiler_re.replace_all(&out, "<tg-spoiler>$1</tg-spoiler>").to_string();

    out = super::blockquotes::convert_blockquotes(&out);

    let hr_re = Regex::new(r"(?m)^[-*]{3,}$").unwrap();
    out = hr_re.replace_all(&out, "———").to_string();

    let bullet_re = Regex::new(r"(?m)^[-*]\s+").unwrap();
    out = bullet_re.replace_all(&out, "• ").to_string();

    out
}
