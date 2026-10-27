//! # Session Manager with JSON Persistence
//!
//! ## Overview
//! Manages active sessions list and coordinates JSON-based persistence to disk. Resolves session keys,
//! handles timezone-based daily resets, and checks session freshness.
//!
//! ## Search Tags
//! #session-registry, #json-persistence, #session-resolving, #freshness-check

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use chrono::Utc;
use tokio::sync::Mutex;
use crate::session::key::SessionKey;
use crate::session::data::SessionData;

/// Type alias `TopicNameResolver` (topic name resolver).
pub type TopicNameResolver = std::sync::Arc<dyn Fn(i64, i64) -> Option<String> + Send + Sync>;

/// Represents `SessionManager` (session manager).
pub struct SessionManager {
    path: PathBuf,
    idle_mins: i64,
    reset_hour: u32,
    reset_enabled: bool,
    tz: String,
    max_msgs: Option<i64>,
    lock: Mutex<()>,
    resolver: Option<TopicNameResolver>,
    /// Lock pool.
    pub lock_pool: std::sync::Arc<crate::bus::lock_pool::LockPool>,
}

fn merge_provider_session(
    cur_data: &mut crate::session::data::ProviderSessionData,
    data: &crate::session::data::ProviderSessionData,
) {
    let same_session = cur_data.session_id == data.session_id;
    if !same_session
        && !cur_data.session_id.is_empty()
        && !cur_data.past_session_ids.contains(&cur_data.session_id)
    {
        cur_data.past_session_ids.push(cur_data.session_id.clone());
    }
    cur_data.session_id = data.session_id.clone();
    for past_id in &data.past_session_ids {
        if !cur_data.past_session_ids.contains(past_id) {
            cur_data.past_session_ids.push(past_id.clone());
        }
    }
    cur_data.message_count = if same_session {
        cur_data.message_count.max(data.message_count)
    } else {
        data.message_count
    };
}

impl SessionManager {
    /// Creates a new instance.
    pub fn new(
        sessions_path: PathBuf,
        idle_timeout_minutes: i64,
        daily_reset_hour: u32,
        daily_reset_enabled: bool,
        user_timezone: String,
        max_session_messages: Option<i64>,
    ) -> Self {
        Self {
            path: sessions_path,
            idle_mins: idle_timeout_minutes,
            reset_hour: daily_reset_hour,
            reset_enabled: daily_reset_enabled,
            tz: user_timezone,
            max_msgs: max_session_messages,
            lock: Mutex::new(()),
            resolver: None,
            lock_pool: std::sync::Arc::new(crate::bus::lock_pool::LockPool::new()),
        }
    }

    /// Executes `is_fresh` (is fresh).
    pub fn is_fresh(&self, session: &SessionData) -> bool {
        super::freshness::is_session_fresh(
            session,
            self.max_msgs,
            self.idle_mins,
            self.reset_enabled,
            self.reset_hour,
            &self.tz,
        )
    }

    /// Executes `load` (load).
    pub fn load(&self) -> Result<HashMap<String, SessionData>, String> {
        if !self.path.exists() { return Ok(HashMap::new()); }
        let content = fs::read_to_string(&self.path).map_err(|e| e.to_string())?;
        if content.trim().is_empty() { return Ok(HashMap::new()); }
        let raw_map: HashMap<String, serde_json::Value> = serde_json::from_str(&content).unwrap_or_default();
        let mut result = HashMap::new();
        for (k, v) in raw_map {
            let Ok(pk) = k.parse::<SessionKey>() else { continue; };
            let Ok(mut sd) = serde_json::from_value::<SessionData>(v) else { continue; };
            if sd.transport.is_empty() { sd.transport = pk.transport.clone(); }
            result.insert(pk.storage_key(), sd);
        }
        Ok(result)
    }

    /// Executes `save` (save).
    pub fn save(&self, sessions: &HashMap<String, SessionData>) -> Result<(), String> {
        let temp_path = self.path.with_extension("tmp");
        let content = serde_json::to_string_pretty(sessions).map_err(|e| e.to_string())?;
        if let Some(p) = self.path.parent() { fs::create_dir_all(p).map_err(|e| e.to_string())?; }
        fs::write(&temp_path, content).map_err(|e| e.to_string())?;
        fs::rename(&temp_path, &self.path).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Executes `get_active` (get active).
    pub async fn get_active(&self, key: &SessionKey) -> Result<Option<SessionData>, String> {
        let _guard = self.lock.lock().await;
        let sessions = self.load()?;
        Ok(sessions.get(&key.storage_key()).cloned())
    }

    /// Executes `resolve_session` (resolve session).
    pub async fn resolve_session(&self, key: &SessionKey, provider: &str, model: &str) -> Result<(SessionData, bool), String> {
        let _guard = self.lock.lock().await;
        let mut sessions = self.load()?;
        let skey = key.storage_key();

        let mut resolved_topic_name = None;
        if let Some(ref resolver) = self.resolver
            && let Some(tid) = key.topic_id
        {
            resolved_topic_name = resolver(key.chat_id, tid);
        }

        if let Some(existing) = sessions.get_mut(&skey) {
            if self.is_fresh(existing) {
                let mut to_save = false;
                if resolved_topic_name.is_some() && existing.topic_name != resolved_topic_name {
                    existing.topic_name = resolved_topic_name;
                    to_save = true;
                }
                let session_id = existing.get_session_id(&existing.provider);
                let cloned = existing.clone();
                if to_save {
                    self.save(&sessions)?;
                }
                return Ok((cloned, session_id.is_empty()));
            }
            existing.clear_provider_session(provider);
            existing.last_active = Utc::now().to_rfc3339();
            existing.last_progress_msg_id = None;
            if resolved_topic_name.is_some() {
                existing.topic_name = resolved_topic_name;
            }
            let cloned = existing.clone();
            self.save(&sessions)?;
            return Ok((cloned, true));
        }

        let mut new_session = SessionData::new(
            key.chat_id,
            key.transport.clone(),
            key.topic_id,
            provider.to_string(),
            model.to_string(),
        );
        new_session.topic_name = resolved_topic_name;
        sessions.insert(skey, new_session.clone());
        self.save(&sessions)?;
        Ok((new_session, true))
    }

    /// Executes `preserve_session_identity` (preserve session identity).
    pub async fn preserve_session_identity(&self, session: &SessionData) -> Result<SessionData, String> {
        let _guard = self.lock.lock().await;
        let mut sessions = self.load()?;
        let skey = session.session_key().storage_key();
        let mut current = sessions.remove(&skey).unwrap_or_else(|| session.clone());
        current.provider = session.provider.clone();
        current.model = session.model.clone();
        current.language = session.language.clone();
        current.effort = session.effort.clone();
        current.last_progress_msg_id = session.last_progress_msg_id;
        current.pending_attachments = session.pending_attachments.clone();
        current.last_handover_summary = session.last_handover_summary.clone();
        if session.topic_name.is_some() && current.topic_name.is_none() {
            current.topic_name = session.topic_name.clone();
        }
        for (prov, data) in &session.provider_sessions {
            let cur_data = current.provider_sessions.entry(prov.clone()).or_default();
            merge_provider_session(cur_data, data);
        }
        current.last_active = Utc::now().to_rfc3339();
        sessions.insert(skey, current.clone());
        self.save(&sessions)?;
        Ok(current)
    }

    /// Executes `update_session` (update session).
    pub async fn update_session(&self, session: &SessionData) -> Result<SessionData, String> {
        let mut cur = self.preserve_session_identity(session).await?;
        let _guard = self.lock.lock().await;
        let mut sessions = self.load()?;
        let skey = cur.session_key().storage_key();
        let act_ps = cur.provider_sessions.entry(cur.provider.clone()).or_default();
        act_ps.message_count += 1;
        sessions.insert(skey, cur.clone());
        self.save(&sessions)?;
        Ok(cur)
    }

    /// Executes `reset_provider_session` (reset provider session).
    pub async fn reset_provider_session(&self, k: &SessionKey, provider: &str, model: &str) -> Result<SessionData, String> {
        let _guard = self.lock.lock().await;
        let mut map = self.load()?;
        let skey = k.storage_key();
        let cur = match map.get_mut(&skey) {
            Some(ex) => {
                ex.clear_provider_session(provider);
                ex.provider = provider.to_string();
                ex.model = model.to_string();
                ex.last_active = Utc::now().to_rfc3339();
                ex.last_handover_summary = None;
                ex.clone()
            }
            None => {
                let n = SessionData::new(k.chat_id, k.transport.clone(), k.topic_id, provider.to_string(), model.to_string());
                map.insert(skey, n.clone());
                n
            }
        };
        self.save(&map)?;
        Ok(cur)
    }

    /// Executes `migrate_chat_id` (migrate chat id).
    pub async fn migrate_chat_id(&self, old: i64, new: i64) -> Result<(), String> {
        let _guard = self.lock.lock().await;
        let mut map = HashMap::new();
        let mut chg = false;
        for (k, mut v) in self.load()? {
            if v.chat_id == old {
                v.chat_id = new;
                let mut pk = k.parse::<SessionKey>()?;
                pk.chat_id = new;
                map.insert(pk.storage_key(), v);
                chg = true;
            } else { map.insert(k, v); }
        }
        if chg { self.save(&map)?; }
        Ok(())
    }

    /// Executes `with_topic_resolver` (with topic resolver).
    pub fn with_topic_resolver(mut self, resolver: TopicNameResolver) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// Executes `list_all` (list all).
    pub async fn list_all(&self) -> Result<Vec<SessionData>, String> {
        let _guard = self.lock.lock().await;
        Ok(self.load()?.into_values().collect())
    }
}
