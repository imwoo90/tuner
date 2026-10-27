//! # Telegram Media Handling Helpers
//!
//! ## Overview
//! Handles identification, download, and prompt injection for media files received via Telegram.
//!
//! ## Search Tags
//! #telegram-media, #media-download, #prompt-injection

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

fn get_media_file_id_and_ext(message: &Message) -> Option<(teloxide::types::FileId, String)> {
    if let Some(photo) = message.photo() {
        if let Some(best_size) = photo.last() {
            return Some((best_size.file.id.clone(), "jpg".to_string()));
        }
    } else if let Some(doc) = message.document() {
        let ext = doc.file_name.as_ref()
            .and_then(|name| name.split('.').next_back())
            .unwrap_or("bin")
            .to_string();
        return Some((doc.file.id.clone(), ext));
    } else if let Some(voice) = message.voice() {
        return Some((voice.file.id.clone(), "ogg".to_string()));
    } else if let Some(audio) = message.audio() {
        return Some((audio.file.id.clone(), "mp3".to_string()));
    } else if let Some(video) = message.video() {
        return Some((video.file.id.clone(), "mp4".to_string()));
    } else if let Some(vn) = message.video_note() {
        return Some((vn.file.id.clone(), "mp4".to_string()));
    } else if let Some(sticker) = message.sticker() {
        return Some((sticker.file.id.clone(), "webp".to_string()));
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

