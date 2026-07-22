//! # Chat Reply and Prompt Context Builder
//!
//! ## Overview
//! Constructs context-rich prompts by parsing reply histories, removing bot mentions,
//! downloading media attachments, and injecting system hints.
//!
//! ## Collaboration Graph
//! - Builds payloads before calling [`runner`](crate::telegram::runner) streams.
//! - Resolves parent messages to inject prompt-context buffers.
//!
//! ## Search Tags
//! #prompt-builder, #reply-history, #media-downloader, #mention-filter

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

pub(crate) use super::media::{has_media, download_telegram_media, download_and_inject_media_hint, prepend_reply_to_media};

fn get_remaining_prompt(rest: &str, consumed_tokens: usize) -> &str {
    let mut remaining_text = "";
    let mut count = 0;
    for (i, ch) in rest.char_indices() {
        if ch.is_whitespace() {
            if i > 0 && !rest.as_bytes()[i-1].is_ascii_whitespace() {
                count += 1;
                if count == consumed_tokens {
                    remaining_text = rest[i..].trim();
                    break;
                }
            }
        }
    }
    if count < consumed_tokens {
        ""
    } else {
        remaining_text
    }
}

pub(crate) fn parse_model_directive(text: &str) -> (Option<String>, Option<String>, &str) {
    let t = text.trim();
    if let Some(r) = t.strip_prefix("@model ") {
        let rest = r.trim();
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if !parts.is_empty() {
            let mut model_name = parts[0].to_string();
            let mut effort = None;
            let mut consumed_tokens = 1;
            
            if parts.len() >= 3 && parts[1] == "--effort" {
                effort = Some(parts[2].to_string());
                consumed_tokens = 3;
            } else if parts.len() >= 2 && (parts[1] == "high" || parts[1] == "medium" || parts[1] == "low") {
                effort = Some(parts[1].to_string());
                consumed_tokens = 2;
            }

            if model_name.ends_with("-high") {
                effort = Some("high".to_string());
                model_name = model_name[..model_name.len() - 5].to_string();
            } else if model_name.ends_with("-medium") {
                effort = Some("medium".to_string());
                model_name = model_name[..model_name.len() - 7].to_string();
            } else if model_name.ends_with("-low") {
                effort = Some("low".to_string());
                model_name = model_name[..model_name.len() - 4].to_string();
            }
            
            let remaining_text = get_remaining_prompt(rest, consumed_tokens);
            return (Some(model_name), effort, remaining_text);
        }
    } else if t.starts_with('@') {
        let dir = t.split_whitespace().next().unwrap_or("");
        let m = &dir[1..];
        if ["opus", "sonnet", "haiku", "gpt-4o", "gpt-4-turbo"].contains(&m) {
            return (Some(m.to_string()), None, t[dir.len()..].trim());
        }
    }
    (None, None, t)
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
        let eff = sess.effort.as_ref().or(config.effort.as_ref());
        if let Some(eff_str) = eff {
            if !eff_str.is_empty() {
                return format!("{} (effort: {})", sess.model, eff_str);
            }
        }
        sess.model
    } else {
        default_model.to_string()
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





