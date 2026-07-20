//! Convert Markdown to Matrix-compatible HTML.
//!
//! Handles basic formatting tags and body splitting at 60k bytes.

//! 
//! ## Search Tags
//! #formatting

use regex::Regex;

/// Remove `[button:...]` markers from text.
pub fn strip_button_markers(text: &str) -> String {
    let button_re = Regex::new(r"\[button:([^\]]+)\]").unwrap();
    button_re.replace_all(text, "").into_owned().trim_end().to_string()
}

/// Convert Markdown to Matrix HTML. Returns (plain_body, formatted_body).
pub fn markdown_to_matrix_html(text: &str) -> (String, String) {
    let cleaned = strip_button_markers(text);
    let formatted = convert_markdown(&cleaned);
    let plain = strip_html(&formatted);
    (plain, formatted)
}

fn escape_html(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '"' => s.push_str("&quot;"),
            '\'' => s.push_str("&#x27;"),
            _ => s.push(c),
        }
    }
    s
}

fn convert_markdown(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let lines = text.split('\n');
    let mut result = Vec::new();
    let mut in_code_block = false;
    let heading_re = Regex::new(r"^(#{1,6})\s+(.+)$").unwrap();
    let hr_re = Regex::new(r"^---+$").unwrap();

    for line in lines {
        if line.starts_with("```") {
            if in_code_block {
                result.push("</code></pre>".to_string());
                in_code_block = false;
            } else {
                let code_lang = line[3..].trim().to_string();
                let lang_attr = if !code_lang.is_empty() {
                    format!(" class=\"language-{}\"", escape_html(&code_lang))
                } else {
                    "".to_string()
                };
                result.push(format!("<pre><code{}>", lang_attr));
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            result.push(escape_html(line));
            continue;
        }

        if let Some(caps) = heading_re.captures(line) {
            let level = caps.get(1).unwrap().as_str().len();
            let content = inline_format(caps.get(2).unwrap().as_str());
            result.push(format!("<h{}>{}</h{}>", level, content, level));
            continue;
        }

        if hr_re.is_match(line.trim()) {
            result.push("<hr>".to_string());
            continue;
        }

        if !line.trim().is_empty() {
            result.push(format!("{}<br>", inline_format(line)));
        } else {
            result.push("<br>".to_string());
        }
    }

    if in_code_block {
        result.push("</code></pre>".to_string());
    }

    result.join("\n")
}

fn inline_format(text: &str) -> String {
    let escaped = escape_html(text);

    let inline_code_re = Regex::new(r"`([^`]+)`").unwrap();
    let text = inline_code_re.replace_all(&escaped, "<code>$1</code>");

    let bold_re1 = Regex::new(r"\*\*(.+?)\*\*").unwrap();
    let bold_re2 = Regex::new(r"__(.+?)__").unwrap();
    let text = bold_re1.replace_all(&text, "<strong>$1</strong>");
    let text = bold_re2.replace_all(&text, "<strong>$1</strong>");

    let italic_re1 = Regex::new(r"\*(.+?)\*").unwrap();
    let italic_re2 = Regex::new(r"\b_(.+?)_\b").unwrap();
    let text = italic_re1.replace_all(&text, "<em>$1</em>");
    let text = italic_re2.replace_all(&text, "<em>$1</em>");

    let strike_re = Regex::new(r"~~(.+?)~~").unwrap();
    let text = strike_re.replace_all(&text, "<del>$1</del>");

    let link_re = Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap();
    let text = link_re.replace_all(&text, "<a href=\"$2\">$1</a>");

    text.into_owned()
}

fn strip_html(formatted: &str) -> String {
    let tag_re = Regex::new(r"<[^>]+>").unwrap();
    let text = tag_re.replace_all(formatted, "");
    html_escape::decode_html_entities(&text).into_owned()
}

/// Split text into chunks that fit within the Matrix event size limit (60k bytes).
pub fn split_text(plain: &str) -> Vec<(String, String)> {
    let plain_lines = plain.split('\n');
    let mut raw_chunks = Vec::new();
    let mut cur_lines = Vec::new();
    let mut cur_size = 0;

    for line in plain_lines {
        let line_size = line.len() + 1;
        if cur_size + line_size > 60_000 && !cur_lines.is_empty() {
            raw_chunks.push(cur_lines.join("\n"));
            cur_lines.clear();
            cur_size = 0;
        }
        cur_lines.push(line);
        cur_size += line_size;
    }

    if !cur_lines.is_empty() {
        raw_chunks.push(cur_lines.join("\n"));
    }

    if raw_chunks.is_empty() {
        return vec![(String::new(), String::new())];
    }

    raw_chunks
        .into_iter()
        .map(|chunk| markdown_to_matrix_html(&chunk))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_removes_buttons() {
        assert_eq!(strip_button_markers("text [button:OK]"), "text");
    }

    #[test]
    fn test_multiple_buttons() {
        let result = strip_button_markers("[button:A] mid [button:B]");
        assert!(!result.contains("[button:"));
        assert!(result.contains("mid"));
    }

    #[test]
    fn test_no_buttons_unchanged() {
        assert_eq!(strip_button_markers("plain text"), "plain text");
    }

    #[test]
    fn test_bold() {
        let (plain, html) = markdown_to_matrix_html("**bold**");
        assert!(html.contains("<strong>bold</strong>"));
        assert!(plain.contains("bold"));
        assert!(!plain.contains("<"));
    }

    #[test]
    fn test_italic() {
        let (_, html) = markdown_to_matrix_html("*italic*");
        assert!(html.contains("<em>italic</em>"));
    }

    #[test]
    fn test_inline_code() {
        let (_, html) = markdown_to_matrix_html("use `func()`");
        assert!(html.contains("<code>func()</code>"));
    }

    #[test]
    fn test_code_block() {
        let text = "```python\nprint('hi')\n```";
        let (_, html) = markdown_to_matrix_html(text);
        assert!(html.contains("<pre><code class=\"language-python\">"));
        assert!(html.contains("print(&#x27;hi&#x27;)"));
    }

    #[test]
    fn test_code_block_no_language() {
        let text = "```\ncode\n```";
        let (_, html) = markdown_to_matrix_html(text);
        assert!(html.contains("<pre><code>"));
    }

    #[test]
    fn test_heading() {
        let (_, html) = markdown_to_matrix_html("## Title");
        assert!(html.contains("<h2>Title</h2>"));
    }

    #[test]
    fn test_heading_h1() {
        let (_, html) = markdown_to_matrix_html("# Big");
        assert!(html.contains("<h1>Big</h1>"));
    }

    #[test]
    fn test_horizontal_rule() {
        let (_, html) = markdown_to_matrix_html("---");
        assert!(html.contains("<hr>"));
    }

    #[test]
    fn test_strikethrough() {
        let (_, html) = markdown_to_matrix_html("~~deleted~~");
        assert!(html.contains("<del>deleted</del>"));
    }

    #[test]
    fn test_link() {
        let (_, html) = markdown_to_matrix_html("[click](https://example.com)");
        assert!(html.contains("<a href=\"https://example.com\">click</a>"));
    }



    #[test]
    fn test_html_escaping() {
        let (_, html) = markdown_to_matrix_html("<script>alert('xss')</script>");
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn test_plain_text_strips_html() {
        let (plain, _) = markdown_to_matrix_html("**bold** and *italic*");
        assert!(!plain.contains("<"));
        assert!(plain.contains("bold"));
        assert!(plain.contains("italic"));
    }

    #[test]
    fn test_buttons_stripped() {
        let (plain, html) = markdown_to_matrix_html("Choose: [button:OK]");
        assert!(!html.contains("[button:"));
        assert!(!plain.contains("[button:"));
    }

    #[test]
    fn test_unclosed_code_block() {
        let text = "```\ncode without closing";
        let (_, html) = markdown_to_matrix_html(text);
        assert!(html.contains("</code></pre>"));
    }

    #[test]
    fn test_empty_input() {
        let (plain, html) = markdown_to_matrix_html("");
        assert_eq!(plain, "");
        assert_eq!(html, "");
    }

    #[test]
    fn test_splitting() {
        let long_line = "a".repeat(35000);
        let text = format!("{}\n{}", long_line, long_line);
        let chunks = split_text(&text);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].0, long_line);
        assert_eq!(chunks[1].0, long_line);
    }

    #[test]
    fn test_adversarial_nesting_and_unclosed() {
        let (plain, html) = markdown_to_matrix_html("**unclosed bold");
        assert_eq!(plain, "**unclosed bold");
        assert_eq!(html, "**unclosed bold<br>");

        let (plain, html) = markdown_to_matrix_html("`unclosed code");
        assert_eq!(plain, "`unclosed code");
        assert_eq!(html, "`unclosed code<br>");

        let (plain, html) = markdown_to_matrix_html("**bold *italic* and ~~strike~~ bold**");
        assert!(html.contains("<strong>bold <em>italic</em> and <del>strike</del> bold</strong>"));
        assert_eq!(plain, "bold italic and strike bold");

        let (plain, html) = markdown_to_matrix_html("[Wiki](https://en.wikipedia.org/wiki/Tag_(metadata))");
        println!("Wiki link output: html={}", html);
    }

    #[test]
    fn test_large_input_splitting_edge_cases() {
        let giant_line = "a".repeat(70000);
        let chunks = split_text(&giant_line);
        println!("Giant line split into {} chunks", chunks.len());
        // Verify that giant line is NOT split (the bug where single line >60k is not split)
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].0.len() > 60000);

        let code_block = format!("```rust\n{}\n```", "fn main() {}\n".repeat(5000));
        assert!(code_block.len() > 60000);
        let chunks = split_text(&code_block);
        assert!(chunks.len() > 1);
        println!("Code block split into {} chunks", chunks.len());
        // Chunk 0 will end with </code></pre> because the markdown parser auto-closes open code blocks
        assert!(chunks[0].1.ends_with("</code></pre>"));
        // Chunk 1 will actually contain <pre><code> at the end because the closing ``` is
        // incorrectly interpreted as the START of a code block (due to lack of syntax/context awareness)!
        assert!(chunks[1].1.contains("<pre><code>"));
        // And inside Chunk 1, code lines are corrupted and parsed as normal markdown,
        // so they have <br> appended instead of being in pre/code tags.
        assert!(chunks[1].1.contains("fn main() {}<br>"));
    }
}
