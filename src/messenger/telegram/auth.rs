//! # Telegram Ingress Authentication and Migration
//!
//! Validates user authorization, handles first-time owner self-registration,
//! group access control filtering, and Telegram forum topic creation/migration events.

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::session::manager::SessionManager;
use super::topic_cache::{self, TopicNameCache};

pub(crate) fn auto_register_owner(from_id: i64) {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let path = std::path::PathBuf::from(&home).join(".tuner/config/config.json");
    if let Ok(c) = std::fs::read_to_string(&path) {
        if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&c) {
            if let Some(obj) = val.as_object_mut() {
                obj.insert("allowed_user_ids".to_string(), serde_json::json!([from_id]));
                if let Ok(pretty) = serde_json::to_string_pretty(&val) {
                    if std::fs::write(&path, pretty).is_ok() {
                        let restart = std::path::PathBuf::from(&home).join(".tuner/restart-requested");
                        let _ = std::fs::write(restart, "");
                    }
                }
            }
        }
    }
}

pub(crate) async fn validate_and_auth_message(
    bot: &Bot,
    msg: &Message,
    config: &CliConfig,
    sessions: &SessionManager,
    topic_cache: &TopicNameCache,
) -> Result<bool, teloxide::RequestError> {
    let from_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);
    let chat_id = msg.chat.id.0;
    if let Some(to_chat) = msg.migrate_to_chat_id() {
        let _ = sessions.migrate_chat_id(chat_id, to_chat.0).await;
        return Ok(false);
    }
    if topic_cache::handle_forum_topic_events(msg, topic_cache, chat_id) {
        return Ok(false);
    }
    let mut ok = config.allowed_user_ids.contains(&from_id);

    if !ok && from_id != 0 && config.allowed_user_ids.is_empty() {
        println!("🤖 [tuner] First-time owner auto-registered! Telegram User ID: {}. Restarting...", from_id);
        let _ = bot.send_message(msg.chat.id, "🤖 Owner registered successfully! Restarting tuner daemon...").await;
        auto_register_owner(from_id);
        std::process::exit(0);
    }

    let is_group = msg.chat.is_group() || msg.chat.is_supergroup();
    if ok && is_group && !config.allowed_group_ids.contains(&chat_id) {
        eprintln!("⚠️ [tuner] Unauthorized group ID: {}", chat_id);
    }
    ok = ok && (!is_group || config.allowed_group_ids.contains(&chat_id));
    Ok(ok)
}
