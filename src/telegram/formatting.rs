//! # Telegram HTML Formatting and Message Splitting
//!
//! This module converts Markdown formatting to Telegram-compatible HTML tags
//! and splits long messages safely without breaking HTML tag pairs.

pub mod helpers;
pub mod splitting;
pub mod italic;
pub mod blockquotes;
pub mod table;
pub mod buttons;

pub use helpers::markdown_to_telegram_html;
pub use splitting::split_html_message;

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
