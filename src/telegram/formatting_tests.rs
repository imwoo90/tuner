//! # Formatting Tests
//!
//! This module contains tests for HTML conversion and chunk splitting of Telegram messages.

#[cfg(test)]
mod tests {
    use crate::telegram::formatting::{markdown_to_telegram_html, split_html_message};

    #[test]
    fn test_markdown_to_telegram_html() {
        let input = "### Hello World\nThis is **bold** and *italic* and ~~strike~~ and ||spoiler||.\n[Google](https://google.com)\n`inline code`\n```rust\nfn main() {}\n```";
        let html = markdown_to_telegram_html(input);

        assert!(html.contains("<b>Hello World</b>"));
        assert!(html.contains("<b>bold</b>"));
        assert!(html.contains("<i>italic</i>"));
        assert!(html.contains("<s>strike</s>"));
        assert!(html.contains("<tg-spoiler>spoiler</tg-spoiler>"));
        assert!(html.contains("<a href=\"https://google.com\">Google</a>"));
        assert!(html.contains("<code>inline code</code>"));
        assert!(html.contains("<pre><code class=\"language-rust\">fn main() {}"));
    }

    #[test]
    fn test_split_html_message_simple() {
        let input = "hello world";
        let chunks = split_html_message(input, 50);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "hello world");
    }

    #[test]
    fn test_split_html_message_keeps_tags_closed() {
        let input = "<b>hello world, this is a very long text that must split</b>";
        let chunks = split_html_message(input, 35);
        
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.starts_with("<b>"));
            assert!(chunk.ends_with("</b>"));
        }
    }

    // --- Phase 1: Bug Fixes and Splitting Enhancements ---

    #[test]
    fn test_hard_split_on_enormous_line() {
        let input = "abcdefghijklmnopqrstuvwxyz";
        let chunks = split_html_message(input, 10);
        assert_eq!(chunks, vec!["abcdefghij", "klmnopqrst", "uvwxyz"]);
    }

    #[test]
    fn test_custom_max_len() {
        let input = "hello world";
        let chunks = split_html_message(input, 5);
        assert_eq!(chunks, vec!["hello", " worl", "d"]);
    }

    #[test]
    fn test_empty_string() {
        let chunks = split_html_message("", 10);
        let expected: Vec<String> = vec![];
        assert_eq!(chunks, expected);
    }

    #[test]
    fn test_nested_tag_safe_splitting() {
        let input = "<b><i>hello world</i></b>";
        // max_len = 16. open tags: <b><i> (6). close tags: </i></b> (8). Total tags len = 14.
        // Capacity per chunk = 2 chars.
        let chunks = split_html_message(input, 16);
        assert_eq!(chunks, vec![
            "<b><i>he</i></b>",
            "<b><i>ll</i></b>",
            "<b><i>o </i></b>",
            "<b><i>wo</i></b>",
            "<b><i>rl</i></b>",
            "<b><i>d</i></b>"
        ]);
    }

    // --- Phase 2: Standard Formatting (List Bullets, Horizontal Rules, and Underscores) ---

    #[test]
    fn test_horizontal_rule() {
        let input = "hello\n---\nworld";
        let html = markdown_to_telegram_html(input);
        assert_eq!(html, "hello\n———\nworld");
    }

    #[test]
    fn test_list_bullet() {
        let input = "- item 1\n* item 2\n-item 3";
        let html = markdown_to_telegram_html(input);
        assert!(html.contains("• item 1"));
        assert!(html.contains("• item 2"));
        assert!(html.contains("-item 3"));
    }

    #[test]
    fn test_underscore_italic() {
        let input = "This is _italic_ and some_variable_name.";
        let html = markdown_to_telegram_html(input);
        assert!(html.contains("<i>italic</i>"));
        assert!(html.contains("some_variable_name"));
    }

    #[test]
    fn test_strict_italic_asterisks() {
        let input = "This is *italic* and this is *not italic because of spaces * and *this is*";
        let html = markdown_to_telegram_html(input);
        assert!(html.contains("<i>italic</i>"));
        assert!(html.contains("*not italic because of spaces *"));
        assert!(html.contains("<i>this is</i>"));
    }

    // --- Phase 3: Blockquotes (Standard & Expandable) ---

    #[test]
    fn test_blockquote() {
        let input = "> this is a quote";
        let html = markdown_to_telegram_html(input);
        assert_eq!(html, "<blockquote>this is a quote</blockquote>");
    }

    #[test]
    fn test_blockquote_expandable() {
        let input = ">! this is expandable";
        let html = markdown_to_telegram_html(input);
        assert_eq!(html, "<blockquote expandable>this is expandable</blockquote>");
    }

    #[test]
    fn test_blockquote_expandable_with_code() {
        let input = ">! ```rust\nfn main() {}\n```";
        let html = markdown_to_telegram_html(input);
        assert_eq!(html, "<blockquote expandable><pre><code class=\"language-rust\">fn main() {}\n</code></pre></blockquote>");
    }

    #[test]
    fn test_consecutive_blockquotes_grouped() {
        let input = "> line 1\n>! line 2\n> line 3";
        let html = markdown_to_telegram_html(input);
        assert_eq!(html, "<blockquote expandable>line 1\nline 2\nline 3</blockquote>");
    }

    // --- Phase 4: Markdown Tables ---

    #[test]
    fn test_table_rendered_as_pre() {
        let input = "| Header 1 | Header 2 |\n|---|---|\n| Cell 1 | Cell 2 |";
        let html = markdown_to_telegram_html(input);
        assert!(html.starts_with("<pre>"));
        assert!(html.ends_with("</pre>"));
        assert!(html.contains("Header 1  Header 2"));
        assert!(html.contains("───────  ────────"));
        assert!(html.contains("Cell 1    Cell 2"));
    }

    // --- Phase 5: Dynamic Button Stripping ---

    #[test]
    fn test_button_syntax_stripped_from_output() {
        let input = "Hello\n\n[button:Label]\n\nWorld";
        let html = markdown_to_telegram_html(input);
        assert!(!html.contains("[button:Label]"));
        assert!(html.contains("Hello"));
        assert!(html.contains("World"));
        assert_eq!(html, "Hello\n\nWorld");
    }

    #[test]
    fn test_button_in_code_block_preserved() {
        let input = "Hello\n```\n[button:Label]\n```\n`[button:Inline]`";
        let html = markdown_to_telegram_html(input);
        assert!(html.contains("[button:Label]"));
        assert!(html.contains("[button:Inline]"));
    }

    // --- Phase 6: Nested Formatting in Links ---

    #[test]
    fn test_nested_formatting_in_link_text() {
        let input = "[**bold link**](https://google.com)";
        let html = markdown_to_telegram_html(input);
        assert_eq!(html, "<a href=\"https://google.com\"><b>bold link</b></a>");
    }
}

