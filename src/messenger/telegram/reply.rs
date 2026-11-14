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
use crate::session::{data::SessionData, manager::SessionManager};

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
#[cfg(test)]
pub(crate) use super::reply_directives::parse_model_directive;

/// Formats the active session's model and effective reasoning effort for `/status`.
pub(crate) async fn resolve_session_model(
    msg: &Message,
    config: &crate::config::CliConfig,
    sessions: &SessionManager,
) -> String {
    use crate::session::data::SessionData;
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, get_topic_id(msg));
    let active = sessions.get_active(&key).await.ok().flatten();
    let raw_prov = active.as_ref().map(|s| s.provider.as_str()).unwrap_or("");
    if !matches!(SessionData::effective_provider(raw_prov, &config.provider), "claude" | "codex") {
        crate::cli::antigravity::model_spec::ensure_live_catalog_fresh().await;
    }
    let (provider, model, display_eff) = SessionData::resolve_display_target(active, config);
    if !super::reply_directives::session_supports_effort(&provider, &model) {
        return model;
    }
    match display_eff {
        Some(eff) => format!("{model} (effort: {eff})"),
        None => model,
    }
}

fn effort_label(val: &str) -> &str {
    match val {
        "minimal" => "Minimal",
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "XHigh",
        "max" => "Max",
        other => other,
    }
}

/// Builds the inline keyboard of reasoning effort levels supported by `(provider, model)`.
pub(crate) fn build_effort_keyboard_for_session(
    provider: &str,
    model: &str,
) -> teloxide::types::InlineKeyboardMarkup {
    use teloxide::types::InlineKeyboardButton as Btn;
    let p = crate::session::data::SessionData::effective_provider(provider, "antigravity");
    let row: Vec<_> = match p {
        "claude" => ["low", "medium", "high", "xhigh", "max"]
            .into_iter()
            .map(|v| Btn::callback(effort_label(v), format!("effort:{p}:{v}")))
            .collect(),
        "codex" => ["minimal", "low", "medium", "high", "xhigh"]
            .into_iter()
            .map(|v| Btn::callback(effort_label(v), format!("effort:{p}:{v}")))
            .collect(),
        _ => crate::cli::antigravity::AntigravityCli::supported_efforts_for_model(model)
            .into_iter()
            .map(|v| Btn::callback(effort_label(&v).to_string(), format!("effort:{p}:{v}")))
            .collect(),
    };
    teloxide::types::InlineKeyboardMarkup::new(vec![row])
}

/// Validates a standalone `effort:<level>` or `effort:<provider>:<level>` callback payload
/// against `(cur_provider, cur_model)`, returning `Ok(level)` or `Err(Some(rejection_markdown))`
/// (or `Err(None)` for malformed unknown strings).
pub(crate) fn validate_standalone_effort_payload(
    payload: &str,
    cur_provider: &str,
    cur_model: &str,
) -> Result<String, Option<String>> {
    let (origin_prov, raw_eff) = match payload.split_once(':') {
        Some((p, e)) => (Some(p.trim()), e.trim()),
        None => (None, payload.trim()),
    };
    let low = raw_eff.to_ascii_lowercase();
    if !super::commands_model::is_valid_effort_level(&low) {
        return Err(None);
    }
    let cur_p = crate::session::data::SessionData::effective_provider(cur_provider, "antigravity");
    if let Some(op) = origin_prov
        && crate::session::data::SessionData::effective_provider(op, cur_p) != cur_p
    {
        return Err(Some(format!(
            "⚠️ Active provider changed to `{cur_p}`. Run `/effort` to refresh."
        )));
    }
    if !super::reply_directives::session_supports_effort(cur_p, cur_model) {
        return Err(Some(format!(
            "ℹ️ Model `{cur_model}` uses fixed reasoning and does not support `/effort` levels."
        )));
    }
    if super::reply_directives::clamp_session_effort(cur_p, cur_model, &low).as_deref()
        != Some(low.as_str())
    {
        return Err(Some(format!(
            "⚠️ Effort `{low}` is not supported for `{cur_model}` on `{cur_p}`. Run `/effort` to refresh."
        )));
    }
    Ok(low)
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





