//! # Telegram Message Chunking and Splitter
//!
//! Splits long reply payloads exceeding Telegram's 4096-character API limit. Automatically
//! balances opened and closed HTML markup tags across split message parts.

use regex::Regex;

fn split_at_char_limit(s: &str, char_limit: usize) -> (&str, &str) {
    if s.is_empty() || char_limit == 0 {
        return ("", s);
    }
    let mut char_count = 0;
    let mut byte_idx = 0;
    for c in s.chars() {
        if char_count >= char_limit {
            break;
        }
        char_count += 1;
        byte_idx += c.len_utf8();
    }
    s.split_at(byte_idx)
}

fn get_tag_name(tag: &str) -> &str {
    let stripped = tag.trim_matches(|c| c == '<' || c == '>');
    stripped.split_whitespace().next().unwrap_or("").trim_start_matches('/')
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

fn split_oversized_token(
    mut token: &str,
    max_len: usize,
    open_tags: &[String],
    current_chunk_parts: &mut Vec<String>,
    current_len: &mut usize,
    chunks: &mut Vec<String>,
) -> String {
    let char_len = |s: &str| s.chars().count();
    let mut closing_tags_str = get_closing_tags(open_tags);
    let mut closing_tags_len = char_len(&closing_tags_str);

    while char_len(token) > (max_len.saturating_sub(*current_len).saturating_sub(closing_tags_len)) {
        let available = max_len.saturating_sub(*current_len).saturating_sub(closing_tags_len);
        let take = if available == 0 { 1 } else { available };
        let (prefix, suffix) = split_at_char_limit(token, take);
        current_chunk_parts.push(prefix.to_string());
        token = suffix;

        current_chunk_parts.push(closing_tags_str.clone());
        chunks.push(current_chunk_parts.join(""));

        *current_chunk_parts = open_tags.to_vec();
        *current_len = open_tags.iter().map(|t| char_len(t)).sum();
        closing_tags_str = get_closing_tags(open_tags);
        closing_tags_len = char_len(&closing_tags_str);
    }
    token.to_string()
}

fn update_tags_and_len(
    token: &str,
    is_tag: bool,
    open_tags: &mut Vec<String>,
    current_chunk_parts: &mut Vec<String>,
    current_len: &mut usize,
) {
    if token.is_empty() {
        return;
    }
    current_chunk_parts.push(token.to_string());
    *current_len += token.chars().count();

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

fn process_token(
    token_ref: &str,
    max_len: usize,
    open_tags: &mut Vec<String>,
    current_chunk_parts: &mut Vec<String>,
    current_len: &mut usize,
    chunks: &mut Vec<String>,
) {
    let char_len = |s: &str| s.chars().count();
    let mut token = token_ref.to_string();
    if token.is_empty() {
        return;
    }

    let is_tag = token.starts_with('<') && token.ends_with('>');
    let closing_tags_str = get_closing_tags(open_tags);
    let closing_tags_len = char_len(&closing_tags_str);

    if !is_tag && char_len(&token) > (max_len.saturating_sub(*current_len).saturating_sub(closing_tags_len)) {
        let available = max_len.saturating_sub(*current_len).saturating_sub(closing_tags_len);
        if available > 0 {
            let (prefix, suffix) = split_at_char_limit(&token, available);
            current_chunk_parts.push(prefix.to_string());
            token = suffix.to_string();
        }

        current_chunk_parts.push(closing_tags_str.clone());
        chunks.push(current_chunk_parts.join(""));

        *current_chunk_parts = open_tags.clone();
        *current_len = open_tags.iter().map(|t| char_len(t)).sum();

        token = split_oversized_token(&token, max_len, open_tags, current_chunk_parts, current_len, chunks);
    } else if !current_chunk_parts.is_empty() && (*current_len + char_len(&token) + closing_tags_len > max_len) {
        current_chunk_parts.push(closing_tags_str);
        chunks.push(current_chunk_parts.join(""));

        *current_chunk_parts = open_tags.clone();
        *current_len = open_tags.iter().map(|t| char_len(t)).sum();
    }

    update_tags_and_len(&token, is_tag, open_tags, current_chunk_parts, current_len);
}

fn has_text_content(s: &str) -> bool {
    let re = Regex::new(r"<[^>]*>").unwrap();
    let text = re.replace_all(s, "");
    !text.trim().is_empty()
}

pub fn split_html_message(text: &str, max_len: usize) -> Vec<String> {
    let char_len = |s: &str| s.chars().count();
    if char_len(text) <= max_len {
        if text.is_empty() {
            return vec![];
        }
        return vec![text.to_string()];
    }

    let tokens = tokenize_html(text);
    let mut chunks = Vec::new();
    let mut current_chunk_parts: Vec<String> = Vec::new();
    let mut current_len = 0;
    let mut open_tags: Vec<String> = Vec::new();

    for token_ref in tokens {
        process_token(
            token_ref,
            max_len,
            &mut open_tags,
            &mut current_chunk_parts,
            &mut current_len,
            &mut chunks,
        );
    }

    if !current_chunk_parts.is_empty() {
        let closing_tags_str = get_closing_tags(&open_tags);
        current_chunk_parts.push(closing_tags_str);
        chunks.push(current_chunk_parts.join(""));
    }

    chunks.into_iter().filter(|c| has_text_content(c)).collect()
}
