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
    fn test_blockquote_expandable_with_fully_quoted_code() {
        let input = ">! ```rust\n>! fn main() {\n>!     println!(\"hello\");\n>! }\n>! ```";
        let html = markdown_to_telegram_html(input);
        assert_eq!(html, "<blockquote expandable><pre><code class=\"language-rust\">fn main() {\n    println!(&quot;hello&quot;);\n}\n</code></pre></blockquote>");
    }

    #[test]
    fn test_blockquote_standard_with_fully_quoted_code() {
        let input = "> ```rust\n> fn main() {}\n> ```";
        let html = markdown_to_telegram_html(input);
        assert_eq!(html, "<blockquote><pre><code class=\"language-rust\">fn main() {}\n</code></pre></blockquote>");
    }

    #[test]
    fn test_unquoted_code_with_greater_than_preserved() {
        let input = "```rust\nif x > 1 {\n    println!(\"> not a quote\");\n}\n```";
        let html = markdown_to_telegram_html(input);
        assert_eq!(html, "<blockquote expandable><pre><code class=\"language-rust\">if x &gt; 1 {\n    println!(&quot;&gt; not a quote&quot;);\n}\n</code></pre></blockquote>");
    }

    #[test]
    fn test_unquoted_code_block_auto_expandable() {
        let input = "```python\nprint('hello world')\n```";
        let html = markdown_to_telegram_html(input);
        assert_eq!(html, "<blockquote expandable><pre><code class=\"language-python\">print(&#x27;hello world&#x27;)\n</code></pre></blockquote>");
    }

    #[test]
    fn test_consecutive_blockquotes_grouped() {
        let input = "> line 1\n>! line 2\n> line 3";
        let html = markdown_to_telegram_html(input);
        assert_eq!(html, "<blockquote expandable>line 1\nline 2\nline 3</blockquote>");
    }

    // --- Phase 4: Markdown Tables ---

    #[test]
    fn test_table_rendered_as_structured_cards() {
        let input = "| Header 1 | Header 2 |\n|---|---|\n| Cell 1 | Cell 2 |";
        let html = markdown_to_telegram_html(input);
        assert!(!html.contains("<pre>"));
        assert!(!html.contains("─"));
        assert_eq!(html, "▸ <b>Cell 1</b>\n• <b>Header 2</b>: Cell 2");
    }

    #[test]
    fn test_table_with_bold_inline_code_and_br_bullets() {
        let input = "\
| 메신저 | 에이전트 적합도 | 장단점 요약 |
| :--- | :---: | :--- |
| **Telegram (현재)** | **★★★★★** | • **유일하게 모든 조건 충족**: 무료 Bot API + 인앱 웹뷰<br>• **단점**: 표(Table) 렌더링 불가, **사내망에서 차단되는 경우가 많음** |
| **Slack (슬랙)** | **★★★☆☆** | • **업무/사내망 친화적**: 스레드 구조와 UI 버튼(`Block Kit`)이 훌륭함<br>• **단점**: 무료 플랜은 **90일 지나면 대화 기록이 다 가려지고** 번거로움 |
| **카카오톡 / WhatsApp** | **★☆☆☆☆** | • **부적합**: 공식 봇 API가 유료이고, **보낸 메시지 수정(스트리밍) 자체가 불가능**함 |";
        let html = markdown_to_telegram_html(input);

        assert!(!html.contains("<pre>"));
        assert!(!html.contains("</pre>"));
        assert!(!html.contains("**"));
        assert!(!html.contains("<br>"));
        assert!(!html.contains("&lt;br&gt;"));
        assert!(html.contains("▸ <b>Telegram (현재)</b>"));
        assert!(html.contains("• <b>에이전트 적합도</b>: <b>★★★★★</b>"));
        assert!(html.contains("• <b>장단점 요약</b>:\n  • <b>유일하게 모든 조건 충족</b>:"));
        assert!(html.contains("  • <b>단점</b>: 표(Table) 렌더링 불가, <b>사내망에서 차단되는 경우가 많음</b>"));
        assert!(html.contains("▸ <b>Slack (슬랙)</b>"));
        assert!(html.contains("<code>Block Kit</code>"));
        assert!(html.contains("• <b>장단점 요약</b>: <b>부적합</b>:"));
        assert_eq!(html.matches("<blockquote expandable>").count(), 2);
        assert_eq!(html.matches("<blockquote>").count(), 1);
    }

    #[test]
    fn test_table_with_links_inline_pipes_and_escaped_pipes() {
        let input = "\
| Item | Command / Link | Notes |
|---|---|---|
| **Search** | [Google](https://google.com) | Use `a | b` or escaped \\| pipe |
| **Source** | [main.rs](file:///home/user/src/main.rs) | Local file link |";
        let html = markdown_to_telegram_html(input);

        assert!(!html.contains("<pre>"));
        assert!(html.contains("▸ <b>Search</b>"));
        assert!(html.contains("• <b>Command &#x2F; Link</b>: <a href=\"https://google.com\">Google</a>"));
        assert!(html.contains("• <b>Notes</b>: Use <code>a | b</code> or escaped | pipe"));
        assert!(html.contains("▸ <b>Source</b>"));
        assert!(html.contains("• <b>Command &#x2F; Link</b>: <code>main.rs</code>"));
    }

    #[test]
    fn test_table_with_double_backtick_inline_code_and_multiple_file_links() {
        let input = "\
복합 표(`**볼드**`, `` `인라인 코드` ``, 셀 내부 파이프 `` `A | B` ``) 테스트:

| 계층 | 스택 | 설명 | 링크 |
| :--- | :---: | :--- | :--- |
| **1. 렌더러** | `Rust` \\| `Regex` | • 파이프 보호: 인라인 코드(`` `a | b` ``) 및 이스케이프(\\|) 보존 | [`table.rs`](file:///home/user/table.rs)<br>[`helpers.rs`](file:///home/user/helpers.rs) |";
        let html = markdown_to_telegram_html(input);

        assert!(!html.contains("file:///home/user"));
        assert!(html.contains("<code>`a | b`</code>"));
        assert!(html.contains("<code>table.rs</code>"));
        assert!(html.contains("<code>helpers.rs</code>"));
        assert!(html.contains("• <b>설명</b>: 파이프 보호: 인라인 코드(<code>`a | b`</code>) 및 이스케이프(|) 보존"));
    }

    #[test]
    fn test_non_table_pipe_lines_preserved_without_separator() {
        let input = "Line 1: a | b | c\nLine 2: d | e | f";
        let html = markdown_to_telegram_html(input);
        assert_eq!(html, "Line 1: a | b | c\nLine 2: d | e | f");
    }

    #[test]
    fn test_long_table_splits_cleanly_across_telegram_chunks() {
        let mut rows = String::from("| Service | Status | Description |\n|---|---|---|\n");
        for i in 0..30 {
            rows.push_str(&format!(
                "| **Service {}** | `ONLINE` | **Healthy**: Node {} is processing requests<br>• Latency under 5ms |\n",
                i, i
            ));
        }
        let html = markdown_to_telegram_html(&rows);
        assert!(!html.contains("<pre>"));
        let chunks = split_html_message(&html, 500);
        assert!(chunks.len() > 2);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= 500);
            assert_eq!(chunk.matches("<b>").count(), chunk.matches("</b>").count());
            assert_eq!(chunk.matches("<code>").count(), chunk.matches("</code>").count());
        }
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

    #[test]
    fn test_split_html_message_preserves_html_entities_across_boundaries() {
        let input = "12345&quot;67890";
        let chunks = split_html_message(input, 8);
        assert_eq!(chunks, vec!["12345", "&quot;67", "890"]);

        for chunk in &chunks {
            assert!(!chunk.contains("&qu") || chunk.contains("&quot;"));
            assert!(!chunk.starts_with("ot;"));
            assert!(!chunk.ends_with("&qu"));
        }
    }

    #[test]
    fn test_bold_italic_and_unmatched_delimiters_never_interleave_tags() {
        let html1 = markdown_to_telegram_html("***bold italic***");
        assert_eq!(html1, "<b><i>bold italic</i></b>");

        let html2 = markdown_to_telegram_html("**foo *bar***");
        assert_eq!(html2, "<b>foo <i>bar</i></b>");

        let html3 = markdown_to_telegram_html("**bold *unmatched** and *other*");
        assert_eq!(html3, "<b>bold *unmatched</b> and <i>other</i>");

        let html4 = markdown_to_telegram_html("**bold _unmatched** and _other_");
        assert_eq!(html4, "<b>bold _unmatched</b> and <i>other</i>");
    }

    #[test]
    fn test_streaming_preview_renders_collapsible_blockquotes() {
        let long_thought = (0..60).map(|i| format!(">! thought line {}", i)).collect::<Vec<_>>().join("\n");
        let delta = format!(
            "💭 **Thinking Process:**\n{}\n\n🛠️ **Tool Calls:**\n>! • `run_command({{\"CommandLine\":\"ls\"}})`",
            long_thought
        );
        let html = crate::telegram::stream::render_streaming_preview_html(&delta);
        assert!(html.contains("💭 <b>Thinking Process:</b>"));
        assert!(html.contains("🛠️ <b>Tool Calls:</b>"));
        assert!(!html.contains("Tool Completions"));
        assert_eq!(html.matches("<blockquote expandable>").count(), 2);
        assert_eq!(html.matches("</blockquote>").count(), 2);
        assert!(!html.contains(">! "));
        assert!(!html.contains("**Thinking Process:**"));
    }

    #[test]
    fn test_carousel_and_local_image_embeds_formatted_cleanly() {
        let input = "````carousel\n![3D Isometric View](/home/user/brain/iso.png)\n<!-- slide -->\n```python\nprint(1)\n```\n````\nSee [main.rs](file:///home/user/src/main.rs)";
        let html = markdown_to_telegram_html(input);
        assert!(html.contains("🖼️ <b>3D Isometric View</b>"));
        assert!(!html.contains("language-carousel"));
        assert!(!html.contains("slide"));
        assert!(!html.contains("/home/user/brain/iso.png"));
        assert!(html.contains("<pre><code class=\"language-python\">print(1)\n</code></pre>"));
        assert!(html.contains("<code>main.rs</code>"));
    }
}

