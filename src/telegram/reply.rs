//! # Chat Reply and Prompt Context Builder
//!
//! Builds context-rich prompt inputs by appending reply histories, extracting file references,
//! downloading media attachments, and cleaning up mentions.

use teloxide::types::Message;
use teloxide::net::Download;
use teloxide::requests::Requester;
use teloxide::payloads::SendMessageSetters;

pub fn get_topic_id(msg: &Message) -> Option<i64> {
    match &msg.kind {
        teloxide::types::MessageKind::Common(c) if c.is_topic_message => msg.thread_id.map(|t| t as i64),
        _ => None,
    }
}

pub(crate) fn strip_mention(text: &str, bot_username: Option<&str>) -> String {
    let u = match bot_username {
        Some(n) if !n.is_empty() => n,
        _ => return text.to_string(),
    };
    let tag = format!("@{}", u.strip_prefix('@').unwrap_or(u));
    let lower_text = text.to_lowercase();
    let lower_tag = tag.to_lowercase();

    if let Some(idx) = lower_text.find(&lower_tag) {
        let before = &text[..idx];
        let after = &text[idx + tag.len()..];
        let trimmed = format!("{}{}", before, after).trim().to_string();
        if trimmed.is_empty() { text.to_string() } else { trimmed }
    } else {
        text.to_string()
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

pub(crate) fn parse_model_directive(text: &str) -> (Option<String>, &str) {
    let t = text.trim();
    if let Some(r) = t.strip_prefix("@model ") {
        let p: Vec<&str> = r.splitn(2, ' ').collect();
        return (Some(p[0].to_string()), p.get(1).unwrap_or(&"").trim());
    } else if t.starts_with('@') {
        let dir = t.split_whitespace().next().unwrap_or("");
        let m = &dir[1..];
        if ["opus", "sonnet", "haiku", "gpt-4o", "gpt-4-turbo"].contains(&m) {
            return (Some(m.to_string()), t[dir.len()..].trim());
        }
    }
    (None, t)
}


pub(crate) async fn load_sessions_cache(
    sessions: &crate::session::manager::SessionManager,
    cache: &crate::telegram::TopicNameCache,
) {
    if let Ok(all) = sessions.list_all().await {
        for s in all {
            if let (Some(tid), Some(tname)) = (s.topic_id, s.topic_name) {
                cache.insert(s.chat_id, tid, tname);
            }
        }
    }
}

pub(crate) async fn resolve_session_model(
    msg: &Message,
    config: &crate::config::CliConfig,
    sessions: &crate::session::manager::SessionManager,
) -> String {
    let topic_id = crate::telegram::get_topic_id(msg);
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, topic_id);
    let default_model = config.model.as_deref().unwrap_or("antigravity-default");
    if let Ok((sess, _)) = sessions.resolve_session(&key, &config.provider, default_model).await {
        sess.model
    } else {
        default_model.to_string()
    }
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

fn find_last_active_session(sess_list: &[crate::session::data::SessionData]) -> Option<crate::session::data::SessionData> {
    sess_list.iter().filter(|s| s.transport == "tg").max_by_key(|s| s.last_active.clone()).cloned()
}

pub(crate) async fn send_startup_notification(
    bot: teloxide::Bot,
    sessions: std::sync::Arc<crate::session::manager::SessionManager>,
) {
    println!("🤖 [tuner] send_startup_notification task started");
    match sessions.list_all().await {
        Ok(sess_list) => {
            println!("🤖 [tuner] Loaded {} sessions for startup notification", sess_list.len());
            if let Some(sess) = find_last_active_session(&sess_list) {
                let active_lang = sess.language.clone().unwrap_or_else(|| "en".to_string());
                crate::i18n::TASK_ACTIVE_LANG.scope(active_lang, async move {
                    let startup_msg = crate::t!("bot.startup_complete");
                    let mut req = bot.send_message(teloxide::types::ChatId(sess.chat_id), startup_msg);
                    if let Some(tid) = sess.topic_id {
                        req = req.message_thread_id(tid as i32);
                    }
                    match req.await {
                        Ok(_) => println!("🤖 [tuner] Startup notification sent successfully to chat_id: {}", sess.chat_id),
                        Err(e) => eprintln!("❌ [tuner] Failed to send startup notification to chat_id {}: {:?}", sess.chat_id, e),
                    }
                }).await;
            } else {
                println!("🤖 [tuner] No active Telegram session found for startup notification");
            }
        }
        Err(e) => {
            eprintln!("❌ [tuner] Failed to list sessions for startup notification: {:?}", e);
        }
    }
}





