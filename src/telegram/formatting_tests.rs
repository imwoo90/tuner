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
}
