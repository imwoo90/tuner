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
