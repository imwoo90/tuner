//! # Telegram Reply Prompt Builder
//!
//! This module helps construct context prompts when a user replies to an existing message.

use teloxide::types::Message;
use teloxide::requests::Requester;

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

pub(crate) fn spawn_restart_watcher(home: String) {
    tokio::spawn(async move {
        let marker = std::path::PathBuf::from(home).join(".tuner/restart-requested");
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
        loop {
            interval.tick().await;
            if marker.exists() {
                let _ = std::fs::remove_file(&marker);
                println!("🤖 [tuner] Restart requested via marker. Exiting...");
                std::process::exit(42);
            }
        }
    });
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


async fn handle_model_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    model: &str,
    sessions: &crate::session::manager::SessionManager,
    config: &crate::config::CliConfig,
) {
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let dm = config.model.as_deref().unwrap_or("antigravity-default");
    if let Ok((mut s, _)) = sessions.resolve_session(&key, &config.provider, dm).await {
        s.model = model.to_string();
        let _ = sessions.update_session(&s, 0.0, 0).await;
        let _ = bot.edit_message_text(msg.chat.id, msg.id, crate::t!("bot.model_switch_success", model = model)).await;
    }
}

async fn handle_lang_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    lang: &str,
    sessions: &crate::session::manager::SessionManager,
    config: &crate::config::CliConfig,
) {
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let dm = config.model.as_deref().unwrap_or("antigravity-default");
    if let Ok((mut s, _)) = sessions.resolve_session(&key, &config.provider, dm).await {
        s.language = Some(lang.to_string());
        let _ = sessions.update_session(&s, 0.0, 0).await;
        crate::i18n::set_language(lang);
        let _ = bot.edit_message_text(msg.chat.id, msg.id, crate::t!("bot.language_switch_success", language = lang)).await;
    }
}

async fn handle_callback_query_inner(
    bot: teloxide::Bot,
    q: teloxide::types::CallbackQuery,
    config: std::sync::Arc<crate::config::CliConfig>,
    sessions: std::sync::Arc<crate::session::manager::SessionManager>,
    cron: std::sync::Arc<crate::cron::manager::CronManager>,
) -> Result<(), teloxide::RequestError> {
    use teloxide::prelude::*;
    if let Some(ref d) = q.data {
        if let Some(ref msg) = q.message {
            if let Some(m) = d.strip_prefix("model:") {
                handle_model_callback(&bot, msg, m, &sessions, &config).await;
            } else if let Some(m) = d.strip_prefix("lang:") {
                handle_lang_callback(&bot, msg, m, &sessions, &config).await;
            } else if d.starts_with("crn:") {
                let _ = crate::telegram::cron_selector::handle_cron_callback(&bot, msg.chat.id, msg.id, d, &cron).await;
            }
        }
        let _ = bot.answer_callback_query(q.id).await;
    }
    Ok(())
}

pub(crate) async fn handle_callback_query(
    bot: teloxide::Bot,
    q: teloxide::types::CallbackQuery,
    config: std::sync::Arc<crate::config::CliConfig>,
    sessions: std::sync::Arc<crate::session::manager::SessionManager>,
    cron: std::sync::Arc<crate::cron::manager::CronManager>,
) -> Result<(), teloxide::RequestError> {
    let mut active_lang = config.language.clone().unwrap_or_else(|| "en".to_string());
    if let Some(ref msg) = q.message {
        let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
        let dm = config.model.as_deref().unwrap_or("antigravity-default");
        if let Ok((sess, _)) = sessions.resolve_session(&key, &config.provider, dm).await {
            active_lang = sess.language.unwrap_or_else(|| config.language.clone().unwrap_or_else(|| "en".to_string()));
        }
    }

    crate::i18n::TASK_ACTIVE_LANG.scope(active_lang, async move {
        handle_callback_query_inner(bot, q, config, sessions, cron).await
    }).await
}

