//! # Session Manager with JSON Persistence
//!
//! ## Overview
//! Manages active sessions list and coordinates JSON-based persistence to disk. Resolves session keys,
//! handles timezone-based daily resets, and checks session freshness.
//!
//! ## Search Tags
//! #session-registry, #json-persistence, #session-resolving, #freshness-check

use std::collections::HashMap;
use std::path::PathBuf;
use chrono::Utc;
use tokio::sync::Mutex;
use crate::session::key::SessionKey;
use crate::session::data::SessionData;
use super::{
    apply_identity_fields, create_seeded_session, heal_session_defaults, load_sessions_from_path,
    reset_existing_topic_session, save_sessions_to_path,
};

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
    pub(crate) lock: Mutex<()>,
    resolver: Option<TopicNameResolver>,
    /// Lock pool.
    pub lock_pool: std::sync::Arc<crate::bus::lock_pool::LockPool>,
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
        self.is_fresh_for_provider(session, "antigravity")
    }

    pub(crate) fn is_fresh_for_provider(&self, session: &SessionData, default_prov: &str) -> bool {
        super::is_session_fresh_for_provider(
            session,
            default_prov,
            self.max_msgs,
            self.idle_mins,
            self.reset_enabled,
            self.reset_hour,
            &self.tz,
        )
    }

    /// Executes `load` (load).
    pub fn load(&self) -> Result<HashMap<String, SessionData>, String> {
        load_sessions_from_path(&self.path)
    }

    /// Executes `save` (save).
    pub fn save(&self, sessions: &HashMap<String, SessionData>) -> Result<(), String> {
        save_sessions_to_path(&self.path, sessions)
    }

    /// Executes `get_active` (get active).
    pub async fn get_active(&self, key: &SessionKey) -> Result<Option<SessionData>, String> {
        let _guard = self.lock.lock().await;
        let sessions = self.load()?;
        Ok(sessions.get(&key.storage_key()).cloned())
    }

    /// Loads or initializes `SessionData` for `key` WITHOUT expiring stale `provider_sessions`.
    pub async fn get_or_init_session(&self, key: &SessionKey, provider: &str, model: &str) -> Result<(SessionData, bool), String> {
        let _guard = self.lock.lock().await;
        let mut sessions = self.load()?;
        let skey = key.storage_key();
        let r_name = self.resolver.as_ref().zip(key.topic_id).and_then(|(r, tid)| r(key.chat_id, tid));
        if let Some(existing) = sessions.get_mut(&skey) {
            let changed = heal_session_defaults(existing, provider, model, r_name);
            let sid_empty = existing.get_session_id(&existing.provider).is_empty();
            let cloned = existing.clone();
            if changed { self.save(&sessions)?; }
            return Ok((cloned, sid_empty));
        }
        let new_session = create_seeded_session(key, provider, model, r_name);
        sessions.insert(skey, new_session.clone());
        self.save(&sessions)?;
        Ok((new_session, true))
    }

    /// Executes `resolve_session` (resolve session).
    pub async fn resolve_session(&self, key: &SessionKey, provider: &str, model: &str) -> Result<(SessionData, bool), String> {
        let _guard = self.lock.lock().await;
        let mut sessions = self.load()?;
        let skey = key.storage_key();
        let resolved_topic_name = self.resolver.as_ref().zip(key.topic_id).and_then(|(r, tid)| r(key.chat_id, tid));

        if let Some(existing) = sessions.get_mut(&skey) {
            if self.is_fresh_for_provider(existing, provider) {
                let to_save = heal_session_defaults(existing, provider, model, resolved_topic_name);
                let sid = existing.get_session_id(&existing.provider);
                let cloned = existing.clone();
                if to_save { self.save(&sessions)?; }
                return Ok((cloned, sid.is_empty()));
            }
            let _ = heal_session_defaults(existing, provider, model, resolved_topic_name);
            let active_prov = existing.provider.clone();
            existing.clear_provider_session(&active_prov);
            existing.last_active = Utc::now().to_rfc3339();
            existing.last_progress_msg_id = None;
            let cloned = existing.clone();
            self.save(&sessions)?;
            return Ok((cloned, true));
        }

        let new_session = create_seeded_session(key, provider, model, resolved_topic_name);
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
        apply_identity_fields(&mut current, session);
        sessions.insert(skey, current.clone());
        self.save(&sessions)?;
        Ok(current)
    }

    /// Executes `update_session` (update session).
    pub async fn update_session(&self, session: &SessionData) -> Result<SessionData, String> {
        let _guard = self.lock.lock().await;
        let mut sessions = self.load()?;
        let skey = session.session_key().storage_key();
        let mut cur = sessions.remove(&skey).unwrap_or_else(|| session.clone());
        apply_identity_fields(&mut cur, session);
        let pk = SessionData::effective_provider(&cur.provider, "antigravity").to_string();
        cur.provider = pk.clone();
        cur.increment_message_count(&pk);
        sessions.insert(skey, cur.clone());
        self.save(&sessions)?;
        Ok(cur)
    }

    /// Atomically mutates an existing session (or inserts `fallback` when absent) under one lock.
    pub async fn mutate_session<F>(&self, key: &SessionKey, fallback: Option<&SessionData>, f: F) -> Result<Option<SessionData>, String>
    where
        F: FnOnce(&mut SessionData) -> bool,
    {
        let _guard = self.lock.lock().await;
        let mut sessions = self.load()?;
        let skey = key.storage_key();
        if !sessions.contains_key(&skey) {
            let Some(init) = fallback else { return Ok(None); };
            sessions.insert(skey.clone(), init.clone());
        }
        let Some(cur) = sessions.get_mut(&skey) else { return Ok(None); };
        if !f(cur) {
            return Ok(Some(cur.clone()));
        }
        let out = cur.clone();
        self.save(&sessions)?;
        Ok(Some(out))
    }

    /// Resets the active (and optional `init_prov`) session ID for `k` while preserving live preferences and inactive provider slots.
    pub async fn reset_topic_sessions(&self, k: &SessionKey, default_prov: &str, default_model: &str, init_prov: Option<&str>, topic_hint: Option<String>) -> Result<SessionData, String> {
        let _guard = self.lock.lock().await;
        let mut map = self.load()?;
        let skey = k.storage_key();
        let r_name = topic_hint.or_else(|| self.resolver.as_ref().zip(k.topic_id).and_then(|(r, tid)| r(k.chat_id, tid)));
        let cur = match map.get_mut(&skey) {
            Some(ex) => {
                reset_existing_topic_session(ex, default_prov, default_model, init_prov, r_name);
                ex.clone()
            }
            None => {
                let n = create_seeded_session(k, default_prov, default_model, r_name);
                map.insert(skey, n.clone());
                n
            }
        };
        self.save(&map)?;
        Ok(cur)
    }

    /// Executes `reset_provider_session` (reset provider session).
    pub async fn reset_provider_session(&self, k: &SessionKey, provider: &str, model: &str) -> Result<SessionData, String> {
        let _guard = self.lock.lock().await;
        let mut map = self.load()?;
        let skey = k.storage_key();
        let eff_prov = SessionData::effective_provider(provider, "antigravity");
        let suf_eff = SessionData::extract_compatible_effort_suffix(eff_prov, model);
        let eff_model = SessionData::default_model_for_provider(eff_prov, Some(model));
        let cur = match map.get_mut(&skey) {
            Some(ex) => {
                ex.clear_provider_session(eff_prov);
                ex.provider = eff_prov.to_string();
                ex.model = eff_model;
                if let Some(se) = suf_eff {
                    ex.effort = Some(se);
                }
                ex.normalize_effort_for_provider(eff_prov);
                ex.last_active = Utc::now().to_rfc3339();
                ex.last_progress_msg_id = None;
                ex.clone()
            }
            None => {
                let n = create_seeded_session(k, eff_prov, model, None);
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
