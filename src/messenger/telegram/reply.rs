//! # Chat Reply and Prompt Context Builder
//!
//! ## Overview
//! Constructs context-rich prompts by parsing reply histories, removing bot mentions,
//! downloading media attachments, and injecting system hints.
//!
//! ## Search Tags
//! #prompt-builder, #reply-history, #media-downloader, #mention-filter

use teloxide::types::{ChatId, Message, MessageId, ThreadId};
use teloxide::requests::Requester;
use teloxide::payloads::SendMessageSetters;
use crate::session::{data::SessionData, key::SessionKey, manager::SessionManager};

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



pub(crate) fn format_forward_header(msg: &Message) -> Option<String> {
    if let Some(user) = msg.forward_from_user() {
        let label = match user.username.as_deref() {
            Some(u) if !u.is_empty() => format!("{} (@{})", user.full_name(), u),
            _ => user.full_name(),
        };
        return Some(format!("[Forwarded message from {}]", label));
    }
    if let Some(chat) = msg.forward_from_chat() {
        let title = chat.title().unwrap_or("Channel/Chat");
        return Some(format!("[Forwarded message from {}]", title));
    }
    if let Some(name) = msg.forward_from_sender_name() {
        return Some(format!("[Forwarded message from {}]", name));
    }
    if msg.forward_date().is_some() {
        return Some("[Forwarded message]".to_string());
    }
    None
}

fn describe_replied_message(replied: &Message) -> Option<String> {
    let text = replied.text().or(replied.caption()).map(str::trim).filter(|s| !s.is_empty());
    let body = text.map(str::to_string).or_else(|| super::media::describe_media_attachment(replied))?;
    match format_forward_header(replied) {
        Some(fwd) => Some(format!("{}\n{}", fwd, body)),
        None => Some(body),
    }
}

pub(crate) fn build_reply_prompt(message: &Message, user_text: &str) -> String {
    let effective_user_text = if let Some(fwd) = format_forward_header(message) {
        format!("{}\n{}", fwd, user_text)
    } else {
        user_text.to_string()
    };

    let cited = message.reply_to_message().and_then(describe_replied_message);
    match cited {
        None => effective_user_text,
        Some(text) => {
            let quoted = text
                .lines()
                .map(|line| format!("> {}", line))
                .collect::<Vec<String>>()
                .join("\n");
            format!(
                "The user is replying to this quoted message:\n{}\n\nThe user's message:\n{}",
                quoted, effective_user_text
            )
        }
    }
}

pub(crate) use super::media::{has_media, download_telegram_media};

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
    if ["opus", "sonnet", "argon", "pro", "flash", "gemini"].contains(&m) {
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
    sessions: &SessionManager,
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
    sessions: &SessionManager,
) -> String {
    let topic_id = get_topic_id(msg);
    let key = SessionKey::telegram(msg.chat.id.0, topic_id);
    let default_model = config.model.as_deref().unwrap_or("antigravity-default");
    let (model, sess_effort) = if let Ok(Some(sess)) = sessions.get_active(&key).await {
        let m = if sess.model.is_empty() { default_model.to_string() } else { sess.model };
        (m, sess.effort)
    } else {
        (default_model.to_string(), None)
    };
    if let Some(eff_str) = sess_effort.as_deref().or(config.effort.as_deref())
        && !eff_str.is_empty()
    {
        return format!("{} (effort: {})", model, eff_str);
    }
    model
}

fn find_last_active_session(sess_list: &[SessionData]) -> Option<SessionData> {
    sess_list.iter().filter(|s| s.transport == "tg").max_by_key(|s| s.last_active.clone()).cloned()
}

pub(crate) async fn send_startup_notification(
    bot: teloxide::Bot,
    sessions: std::sync::Arc<SessionManager>,
) {
    println!("🤖 [tuner] send_startup_notification task started");
    match sessions.list_all().await {
        Ok(sess_list) => {
            println!("🤖 [tuner] Loaded {} sessions for startup notification", sess_list.len());
            if let Some(sess) = find_last_active_session(&sess_list) {
                let active_lang = sess.language.clone().unwrap_or_else(crate::i18n::get_language);
                crate::i18n::TASK_ACTIVE_LANG.scope(active_lang, async move {
                    let startup_msg = crate::t!("bot.startup_complete");
                    let chat_id = ChatId(sess.chat_id);
                    let mut req = bot.send_message(chat_id, startup_msg.clone());
                    if let Some(tid) = sess.topic_id {
                        req = req.message_thread_id(ThreadId(MessageId(tid as i32)));
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





