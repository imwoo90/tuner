//! # Telegram Reply Prompt Builder
//!
//! This module helps construct context prompts when a user replies to an existing message.

use teloxide::types::Message;

pub(crate) fn build_reply_prompt(message: &Message, user_text: &str) -> String {
    let cited = if let Some(replied) = message.reply_to_message() {
        replied.text().or(replied.caption()).map(|s| s.trim())
    } else {
        None
    };

    match cited {
        None => user_text.to_string(),
        Some("") => user_text.to_string(),
        Some(text) => {
            let quoted = text
                .lines()
                .map(|line| format!("> {}", line))
                .collect::<Vec<String>>()
                .join("\n");
            format!(
                "The user is replying to this quoted message:\n{}\n\nThe user's message:\n{}",
                quoted, user_text
            )
        }
    }
}
