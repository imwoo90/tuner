//! # Telegram Reply Prompt Builder
//!
//! This module helps construct context prompts when a user replies to an existing message.

use teloxide::types::Message;

pub(crate) fn strip_mention(text: &str, bot_username: Option<&str>) -> String {
    let username = match bot_username {
        Some(name) if !name.is_empty() => name,
        _ => return text.to_string(),
    };
    let username_clean = username.strip_prefix('@').unwrap_or(username);
    let tag = format!("@{}", username_clean);
    let lower_text = text.to_lowercase();
    let lower_tag = tag.to_lowercase();

    if let Some(idx) = lower_text.find(&lower_tag) {
        let before = &text[..idx];
        let after = &text[idx + tag.len()..];
        let stripped = format!("{}{}", before, after);
        let trimmed = stripped.trim().to_string();
        if trimmed.is_empty() {
            text.to_string()
        } else {
            trimmed
        }
    } else {
        text.to_string()
    }
}

fn reply_attachment_label(message: &Message) -> &'static str {
    if message.photo().is_some() {
        "an image"
    } else if message.document().is_some() {
        "a document"
    } else if message.voice().is_some() {
        "a voice message"
    } else if message.audio().is_some() {
        "an audio file"
    } else if message.video().is_some() {
        "a video"
    } else if message.video_note().is_some() {
        "a video note"
    } else if message.sticker().is_some() {
        "a sticker"
    } else {
        "a file"
    }
}

pub(crate) fn prepend_reply_to_media(message: &Message, media_prompt: &str) -> String {
    let cited = if let Some(replied) = message.reply_to_message() {
        replied.text().or(replied.caption()).map(|s| s.trim())
    } else {
        None
    };

    match cited {
        None => media_prompt.to_string(),
        Some("") => media_prompt.to_string(),
        Some(text) => {
            let quoted = text
                .lines()
                .map(|line| format!("> {}", line))
                .collect::<Vec<String>>()
                .join("\n");
            let label = reply_attachment_label(message);
            format!(
                "The user is replying to this quoted message:\n{}\n\nTheir reply is {} (the attached file below).\n\n{}",
                quoted, label, media_prompt
            )
        }
    }
}

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

pub(crate) fn has_media(message: &Message) -> bool {
    message.photo().is_some()
        || message.document().is_some()
        || message.voice().is_some()
        || message.audio().is_some()
        || message.video().is_some()
        || message.video_note().is_some()
        || message.sticker().is_some()
}
