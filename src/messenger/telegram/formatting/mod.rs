//! # Telegram HTML Formatting and Message Splitting
//!
//! ## Overview
//! Converts Markdown formatting to Telegram-compatible HTML tags and splits long messages
//! safely across chunk boundaries without breaking HTML tag pairs.
//!
//! ## Submodules
//! - [`helpers`] (`helpers.rs`): Converts Markdown syntax (links, code blocks, bold) to HTML.
//! - [`splitting`] (`splitting.rs`): Truncates and chunks HTML payloads to comply with Telegram length limits.
//! - [`italic`] (`italic.rs`): Standardizes italic markdown syntax without mangling snake_case identifiers.
//! - [`blockquotes`] (`blockquotes.rs`): Groups blockquote lines into standard and expandable HTML tags.
//! - [`table`] (`table.rs`): Formats Markdown pipe-tables into aligned monospace blocks.
//! - [`buttons`] (`buttons.rs`): Extracts and strips inline button markup tags from response text.
//!
//! ## Search Tags
//! #formatting-index, #markdown-to-html, #message-splitting, #ui-formatting

pub mod helpers;
pub mod splitting;
pub mod italic;
pub mod blockquotes;
pub mod table;
pub mod buttons;

pub use helpers::markdown_to_telegram_html;
pub use splitting::split_html_message;

/// Executes `find_best_option` (find best option).
pub fn find_best_option(text: &str, options: &[String]) -> Option<usize> {
    let t = text.to_lowercase();
    let words: std::collections::HashSet<_> = t.split(|c: char| !c.is_alphanumeric()).filter(|s| !s.is_empty()).collect();
    if words.is_empty() { return None; }
    let mut best = None;
    let mut max_score = 0;
    for (i, opt) in options.iter().enumerate() {
        let o = opt.to_lowercase();
        if o.contains(&t) || t.contains(&o) { return Some(i); }
        let o_words: std::collections::HashSet<_> = o.split(|c: char| !c.is_alphanumeric()).filter(|s| !s.is_empty()).collect();
        let score = words.intersection(&o_words).count();
        if score > max_score {
            max_score = score;
            best = Some(i);
        }
    }
    (max_score > 0).then_some(best).flatten()
}
