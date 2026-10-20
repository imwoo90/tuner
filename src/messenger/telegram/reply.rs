//! # Chat Reply and Prompt Context Builder
//!
//! ## Overview
//! Constructs context-rich prompts by parsing reply histories, removing bot mentions,
//! downloading media attachments, and injecting system hints.
//!
//! ## Search Tags
//! #prompt-builder, #reply-history, #media-downloader, #mention-filter

use teloxide::types::Message;
use teloxide::requests::Requester;
use teloxide::payloads::SendMessageSetters;

/// Executes `get_topic_id` (get topic id).
pub fn get_topic_id(msg: &Message) -> Option<i64> {
    if msg.is_topic_message {
        msg.thread_id.map(|t| t.0.0 as i64)
    } else {
        None
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

pub(crate) use super::media::{has_media, download_telegram_media};
#[cfg(test)]
pub(crate) use super::media::prepend_reply_to_media;

fn get_remaining_prompt(rest: &str, consumed_tokens: usize) -> &str {
    let mut remaining_text = "";
    let mut count = 0;
    for (i, ch) in rest.char_indices() {
        if ch.is_whitespace()
            && i > 0 && !rest.as_bytes()[i-1].is_ascii_whitespace() {
                count += 1;
                if count == consumed_tokens {
                    remaining_text = rest[i..].trim();
                    break;
                }
            }
    }
    if count < consumed_tokens {
        ""
    } else {
        remaining_text
    }
}

fn normalize_model_alias(name: &str) -> (String, Option<String>) {
    match name.to_lowercase().as_str() {
        "opus" | "claude-opus" | "opus-5-5" | "opus5.5" => ("claude-opus-5-5".to_string(), Some("high".to_string())),
        "sonnet" | "claude-sonnet" | "sonnet-5-5" | "sonnet5.5" => ("claude-sonnet-5-5".to_string(), Some("high".to_string())),
        "argon" | "pro" | "gemini-argon" | "gemini-4-argon" | "gemini-3.1-pro" => ("gemini-4-argon".to_string(), Some("high".to_string())),
        "flash" | "gemini" | "gemini-3.5-flash" | "gemini-3.6-flash" | "gemini-3.7-flash" => ("gemini-3.8-flash".to_string(), Some("high".to_string())),
        _ => (name.to_string(), None),
    }
}

fn parse_at_shortcut(t: &str) -> Option<(Option<String>, Option<String>, &str)> {
    if !t.starts_with('@') { return None; }
    let dir = t.split_whitespace().next().unwrap_or("");
    let m = &dir[1..];
    if ["opus", "sonnet", "argon", "pro", "flash", "haiku", "gpt-4o", "gpt-4-turbo"].contains(&m) {
        let (canonical, default_eff) = normalize_model_alias(m);
        return Some((Some(canonical), default_eff, t[dir.len()..].trim()));
    }
    None
}

fn split_legacy_model_effort(m: &str) -> (String, Option<String>) {
    for (suffix, eff) in [("-high", "high"), ("-medium", "medium"), ("-low", "low")] {
        if let Some(prefix) = m.strip_suffix(suffix) {
            return (prefix.to_string(), Some(eff.to_string()));
        }
    }
    (m.to_string(), None)
}

pub(crate) fn parse_model_directive(text: &str) -> (Option<String>, Option<String>, &str) {
    let t = text.trim();
    if let Some(r) = t.strip_prefix("@model ") {
        let rest = r.trim();
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if !parts.is_empty() {
            let (mut model_name, suffix_eff) = split_legacy_model_effort(parts[0]);
            let mut effort = suffix_eff;
            let mut consumed_tokens = 1;
            if parts.len() >= 3 && parts[1] == "--effort" {
                effort = Some(parts[2].to_string());
                consumed_tokens = 3;
            } else if parts.len() >= 2 && matches!(parts[1], "high" | "medium" | "low") {
                effort = Some(parts[1].to_string());
                consumed_tokens = 2;
            }
            let (canonical, default_eff) = normalize_model_alias(&model_name);
            model_name = canonical;
            if effort.is_none() {
                effort = default_eff;
            }
            return (Some(model_name), effort, get_remaining_prompt(rest, consumed_tokens));
        }
    } else if let Some(res) = parse_at_shortcut(t) {
        return res;
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
        if let Some(eff_str) = eff
            && !eff_str.is_empty() {
                return format!("{} (effort: {})", sess.model, eff_str);
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
                    let chat_id = teloxide::types::ChatId(sess.chat_id);
                    let mut req = bot.send_message(chat_id, startup_msg.clone());
                    if let Some(tid) = sess.topic_id {
                        req = req.message_thread_id(teloxide::types::ThreadId(teloxide::types::MessageId(tid as i32)));
                    }
                    match req.await {
                        Ok(_) => println!("🤖 [tuner] Startup notification sent successfully to chat_id: {}", sess.chat_id),
                        Err(e) if sess.topic_id.is_some() && e.to_string().to_lowercase().contains("thread not found") => {
                            let _ = bot.send_message(chat_id, startup_msg).await;
                        }
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





