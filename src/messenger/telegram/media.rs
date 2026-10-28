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

pub(crate) fn describe_media_attachment(msg: &Message) -> Option<String> {
    if let Some(doc) = msg.document() {
        Some(format!("[Attached document: {}]", doc.file_name.as_deref().unwrap_or("file")))
    } else if msg.photo().is_some() {
        Some("[Attached photo]".to_string())
    } else if msg.voice().is_some() {
        Some("[Attached voice note]".to_string())
    } else if let Some(aud) = msg.audio() {
        Some(format!("[Attached audio: {}]", aud.file_name.as_deref().unwrap_or("audio")))
    } else if let Some(vid) = msg.video() {
        Some(format!("[Attached video: {}]", vid.file_name.as_deref().unwrap_or("video")))
    } else if msg.video_note().is_some() {
        Some("[Attached video note]".to_string())
    } else {
        None
    }
}

pub(crate) fn sanitize_filename(raw: &str) -> String {
    let base = std::path::Path::new(raw)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(raw);
    let mut out = String::with_capacity(base.len());
    let mut prev_sep = false;
    for ch in base.chars() {
        if ch.is_alphanumeric() || ch == '-' {
            out.push(ch);
            prev_sep = false;
        } else if ch == '.' {
            if !out.ends_with('.') {
                out.push('.');
            }
            prev_sep = true;
        } else if !prev_sep {
            out.push('_');
            prev_sep = true;
        }
    }
    let trimmed = out.trim_matches(|c: char| c == '.' || c == '_');
    if trimmed.chars().count() <= 64 {
        return trimmed.to_string();
    }
    if let Some((stem, ext)) = trimmed.rsplit_once('.')
        && !ext.is_empty()
        && ext.chars().count() <= 12
    {
        let keep = 63usize.saturating_sub(ext.chars().count());
        let short_stem: String = stem.chars().take(keep).collect();
        return format!("{}.{}", short_stem.trim_end_matches(['.', '_']), ext);
    }
    trimmed.chars().take(64).collect()
}

fn resolve_named_suffix(file_name: Option<&String>, default_ext: &str) -> String {
    if let Some(raw) = file_name {
        let clean = sanitize_filename(raw);
        if !clean.is_empty() {
            return format!("_{}", clean);
        }
    }
    format!(".{}", default_ext)
}

fn get_media_file_id_and_suffix(message: &Message) -> Option<(teloxide::types::FileId, String)> {
    if let Some(photo) = message.photo() {
        if let Some(best_size) = photo.last() {
            return Some((best_size.file.id.clone(), ".jpg".to_string()));
        }
    } else if let Some(doc) = message.document() {
        return Some((doc.file.id.clone(), resolve_named_suffix(doc.file_name.as_ref(), "bin")));
    } else if let Some(voice) = message.voice() {
        return Some((voice.file.id.clone(), ".ogg".to_string()));
    } else if let Some(audio) = message.audio() {
        return Some((audio.file.id.clone(), resolve_named_suffix(audio.file_name.as_ref(), "mp3")));
    } else if let Some(video) = message.video() {
        return Some((video.file.id.clone(), resolve_named_suffix(video.file_name.as_ref(), "mp4")));
    } else if let Some(vn) = message.video_note() {
        return Some((vn.file.id.clone(), ".mp4".to_string()));
    } else if let Some(sticker) = message.sticker() {
        return Some((sticker.file.id.clone(), ".webp".to_string()));
    }
    None
}

pub(crate) async fn download_telegram_media(
    bot: &teloxide::Bot,
    message: &Message,
    dest_dir: &std::path::Path,
) -> Result<Option<String>, String> {
    let (file_id, suffix) = match get_media_file_id_and_suffix(message) {
        Some(res) => res,
        None => return Ok(None),
    };
    let filename = format!("media_{}_{}{}", message.chat.id, message.id, suffix);
    let dest_path = dest_dir.join(&filename);
    if dest_path.is_file() && std::fs::metadata(&dest_path).map(|m| m.len() > 0).unwrap_or(false) {
        return Ok(Some(format!("telegram_files/{}", filename)));
    }
    if cfg!(test) {
        return Ok(Some(format!("telegram_files/mock_media_{}{}", message.id.0, suffix)));
    }

    let file = bot.get_file(file_id).await.map_err(|e| e.to_string())?;
    tokio::fs::create_dir_all(dest_dir).await.map_err(|e| e.to_string())?;

    let mut dest_file = tokio::fs::File::create(&dest_path).await.map_err(|e| e.to_string())?;
    bot.download_file(&file.path, &mut dest_file).await.map_err(|e| e.to_string())?;

    Ok(Some(format!("telegram_files/{}", filename)))
}

pub(crate) fn inject_pre_downloaded_files(prompt: &mut String, files: &[String]) {
    if !files.is_empty() {
        let mut media_hint = String::new();
        if files.len() == 1 {
            media_hint = format!(
                "[SYSTEM HINT] The user attached a file. You can read/view it by calling `view_file` at path: `{}`\n\n",
                files[0]
            );
        } else {
            media_hint.push_str("[SYSTEM HINT] The user attached multiple files. You can view them at:\n");
            for f in files {
                media_hint.push_str(&format!("- `{}`\n", f));
            }
            media_hint.push('\n');
        }
        *prompt = format!("{}{}", media_hint, prompt);
    }
}

pub(crate) async fn queue_pending_files(
    msg: &Message,
    files: &[String],
    config: &crate::config::CliConfig,
    sessions: &crate::session::manager::SessionManager,
) {
    if files.is_empty() {
        return;
    }
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, super::get_topic_id(msg));
    let default_model = config.model.as_deref().unwrap_or("antigravity-default");
    if let Ok((mut sess, _)) = sessions.resolve_session(&key, &config.provider, default_model).await {
        for f in files {
            if !sess.pending_attachments.contains(f) {
                sess.pending_attachments.push(f.clone());
            }
        }
        let _ = sessions.preserve_session_identity(&sess).await;
    }
}

pub(crate) async fn drain_session_attachments(
    bot: &teloxide::Bot,
    msg: &Message,
    config: &crate::config::CliConfig,
    sess: &mut crate::session::data::SessionData,
    pre_downloaded_files: &[String],
    sessions: &crate::session::manager::SessionManager,
) -> Vec<String> {
    let mut all_files = if !sess.pending_attachments.is_empty() {
        let drained = std::mem::take(&mut sess.pending_attachments);
        let _ = sessions.preserve_session_identity(sess).await;
        drained
    } else {
        Vec::new()
    };
    if let Some(replied) = msg.reply_to_message()
        && has_media(replied)
    {
        let dest_dir = config.working_dir.join("telegram_files");
        if let Ok(Some(replied_file)) = download_telegram_media(bot, replied, &dest_dir).await
            && !all_files.contains(&replied_file)
        {
            all_files.push(replied_file);
        }
    }
    for f in pre_downloaded_files {
        if !all_files.contains(f) {
            all_files.push(f.clone());
        }
    }
    all_files
}

pub(crate) async fn build_prompt_with_attachments(
    bot: &teloxide::Bot,
    msg: &Message,
    escaped_text: &str,
    files: &[String],
    config: &crate::config::CliConfig,
    sess: &mut crate::session::data::SessionData,
    sessions: &crate::session::manager::SessionManager,
) -> String {
    let all_files = drain_session_attachments(bot, msg, config, sess, files, sessions).await;
    let mut prompt = super::build_reply_prompt(msg, escaped_text);
    inject_pre_downloaded_files(&mut prompt, &all_files);
    prompt
}

