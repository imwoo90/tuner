//! # Telegram Media Handling Helpers
//!
//! Handles identification, download, and prompt injection for media files received via Telegram.

use teloxide::types::Message;
use teloxide::net::Download;
use teloxide::requests::Requester;

pub(crate) fn has_media(message: &Message) -> bool {
    message.photo().is_some()
        || message.document().is_some()
        || message.voice().is_some()
        || message.audio().is_some()
        || message.video().is_some()
        || message.video_note().is_some()
        || message.sticker().is_some()
}

fn get_media_file_id_and_ext(message: &Message) -> Option<(String, String)> {
    if let Some(photo) = message.photo() {
        if let Some(best_size) = photo.last() {
            return Some((best_size.file.id.clone(), "jpg".to_string()));
        }
    } else if let Some(doc) = message.document() {
        let ext = doc.file_name.as_ref()
            .and_then(|name| name.split('.').last())
            .unwrap_or("bin")
            .to_string();
        return Some((doc.file.id.clone(), ext));
    } else if let Some(voice) = message.voice() {
        return Some((voice.file.id.clone(), "ogg".to_string()));
    } else if let Some(audio) = message.audio() {
        return Some((audio.file.id.clone(), "mp3".to_string()));
    } else if let Some(video) = message.video() {
        return Some((video.file.id.clone(), "mp4".to_string()));
    }
    None
}

pub(crate) async fn download_telegram_media(
    bot: &teloxide::Bot,
    message: &Message,
    dest_dir: &std::path::Path,
) -> Result<Option<String>, String> {
    if cfg!(test) {
        let ext = get_media_file_id_and_ext(message)
            .map(|(_, e)| e)
            .unwrap_or_else(|| "jpg".to_string());
        return Ok(Some(format!("telegram_files/mock_media_{}.{}", message.id.0, ext)));
    }
    let (file_id, ext) = match get_media_file_id_and_ext(message) {
        Some(res) => res,
        None => return Ok(None),
    };

    let file = bot.get_file(file_id).await.map_err(|e| e.to_string())?;
    tokio::fs::create_dir_all(dest_dir).await.map_err(|e| e.to_string())?;

    let filename = format!("media_{}_{}.{}", message.chat.id, message.id, ext);
    let dest_path = dest_dir.join(&filename);

    let mut dest_file = tokio::fs::File::create(&dest_path).await.map_err(|e| e.to_string())?;
    bot.download_file(&file.path, &mut dest_file).await.map_err(|e| e.to_string())?;

    Ok(Some(format!("telegram_files/{}", filename)))
}

pub(crate) async fn download_and_inject_media_hint(
    bot: &teloxide::Bot,
    message: &Message,
    working_dir: &std::path::Path,
    prompt: &mut String,
) -> Result<(), String> {
    if has_media(message) {
        let dest_dir = working_dir.join("telegram_files");
        match download_telegram_media(bot, message, &dest_dir).await {
            Ok(Some(relative_path)) => {
                let media_hint = format!(
                    "[SYSTEM HINT] The user attached a file. You can read/view it by calling `view_file` at path: `{}`\n\n",
                    relative_path
                );
                *prompt = format!("{}{}", media_hint, prompt);
            }
            Ok(None) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
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
            
            let base_type = if message.photo().is_some() {
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
                "a media file"
            };
            let media_type = format!("{} (the attached file below).", base_type);

            format!(
                "The user is replying to this quoted message:\n{}\n\nTheir reply is {}\n\n{}",
                quoted, media_type, media_prompt
            )
        }
    }
}
