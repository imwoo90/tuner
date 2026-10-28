//! # Telegram Ingress Authentication, Migration, and Topic Cache
//!
//! ## Overview
//! Validates user authorization, handles first-time owner self-registration,
//! group access control filtering, and Telegram forum topic metadata caching.
//!
//! ## Search Tags
//! #telegram-auth, #owner-registration, #topic-migration, #topic-cache

use std::collections::HashMap;
use std::sync::Mutex;
use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::session::manager::SessionManager;

#[derive(Clone, Debug)]
/// Represents `BotInfo` (bot info).
pub struct BotInfo {
    /// Username.
    pub username: Option<String>,
}

#[derive(Clone)]
/// Represents `TopicNameCache` (topic name cache).
pub struct TopicNameCache {
    names: std::sync::Arc<Mutex<HashMap<(i64, i64), String>>>,
}

impl Default for TopicNameCache {
    fn default() -> Self {
        Self::new()
    }
}

impl TopicNameCache {
    /// Creates a new instance.
    pub fn new() -> Self {
        Self {
            names: std::sync::Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Executes `insert` (insert).
    pub fn insert(&self, chat_id: i64, topic_id: i64, name: String) {
        let mut map = self.names.lock().unwrap_or_else(|e| e.into_inner());
        map.insert((chat_id, topic_id), name);
    }

    /// Executes `find_by_id` (find by id).
    pub fn find_by_id(&self, chat_id: i64, topic_id: i64) -> Option<String> {
        let map = self.names.lock().unwrap_or_else(|e| e.into_inner());
        map.get(&(chat_id, topic_id)).cloned()
    }

    /// Executes `find_by_name` (find by name).
    pub fn find_by_name(&self, chat_id: i64, name: &str) -> Option<i64> {
        let target_name = name.strip_prefix('@').unwrap_or(name);
        let map = self.names.lock().unwrap_or_else(|e| e.into_inner());
        for (&(cid, tid), tname) in map.iter() {
            if cid == chat_id && tname.eq_ignore_ascii_case(target_name) {
                return Some(tid);
            }
        }
        None
    }
}

pub(crate) fn handle_forum_topic_events(
    msg: &teloxide::types::Message,
    cache: &TopicNameCache,
    chat_id: i64,
) -> bool {
    let tid = match msg.thread_id {
        Some(t) => t.0.0 as i64,
        None => return false,
    };
    match &msg.kind {
        teloxide::types::MessageKind::ForumTopicCreated(c) => {
            cache.insert(chat_id, tid, c.forum_topic_created.name.clone());
            true
        }
        teloxide::types::MessageKind::ForumTopicEdited(e) => {
            if let Some(ref name) = e.forum_topic_edited.name {
                cache.insert(chat_id, tid, name.clone());
            }
            true
        }
        _ => false,
    }
}

pub(crate) fn auto_register_owner(from_id: i64) {
    let tuner_home = std::env::var("TUNER_HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            std::path::PathBuf::from(home).join(".tuner")
        });
    let path = tuner_home.join("config/config.json");
    if let Ok(c) = std::fs::read_to_string(&path)
        && let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&c)
        && let Some(obj) = val.as_object_mut()
    {
        obj.insert("allowed_user_ids".to_string(), serde_json::json!([from_id]));
        if let Ok(pretty) = serde_json::to_string_pretty(&val)
            && std::fs::write(&path, pretty).is_ok()
        {
            let restart = tuner_home.join("restart-requested");
            let _ = std::fs::write(restart, "");
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
    if handle_forum_topic_events(msg, topic_cache, chat_id) {
        return Ok(false);
    }
    let mut ok = config.allowed_user_ids.contains(&from_id);

    if !ok && from_id != 0 && config.allowed_user_ids.is_empty() {
        println!(
            "🤖 [tuner] First-time owner auto-registered! Telegram User ID: {}. Restarting...",
            from_id
        );
        let _ = bot
            .send_message(
                msg.chat.id,
                "🤖 Owner registered successfully! Restarting tuner daemon...",
            )
            .await;
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
