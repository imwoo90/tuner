//! # Session Manager with JSON Persistence
//!
//! This module manages the lifecycle, freshness check, reset, and storage of sessions.
//! It supports legacy keys migration and user timezone daily resets.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use chrono::Utc;
use tokio::sync::Mutex;
use crate::session::key::SessionKey;
use crate::session::data::SessionData;

pub type TopicNameResolver = std::sync::Arc<dyn Fn(i64, i64) -> Option<String> + Send + Sync>;

pub struct SessionManager {
    path: PathBuf,
    idle_mins: i64,
    reset_hour: u32,
    reset_enabled: bool,
    tz: String,
    max_msgs: Option<i64>,
    lock: Mutex<()>,
    resolver: Option<TopicNameResolver>,
}

impl SessionManager {
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
        }
    }

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

    pub fn load(&self) -> Result<HashMap<String, SessionData>, String> {
        if !self.path.exists() { return Ok(HashMap::new()); }
        let content = fs::read_to_string(&self.path).map_err(|e| e.to_string())?;
        if content.trim().is_empty() { return Ok(HashMap::new()); }
        let raw_map: HashMap<String, serde_json::Value> = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Warning: sessions.json corrupt: {}", e);
                return Ok(HashMap::new());
            }
        };
        let mut result = HashMap::new();
        for (k, v) in raw_map {
            let pk = match k.parse::<SessionKey>() {
                Ok(k) => k,
                Err(_) => return Ok(HashMap::new()),
            };
            let mut sd: SessionData = match serde_json::from_value(v) {
                Ok(d) => d,
                Err(_) => return Ok(HashMap::new()),
            };
            if sd.transport.is_empty() { sd.transport = pk.transport.clone(); }
            sd.migrate_legacy_metrics();
            result.insert(pk.storage_key(), sd);
        }
        Ok(result)
    }

    pub fn save(&self, sessions: &HashMap<String, SessionData>) -> Result<(), String> {
        let temp_path = self.path.with_extension("tmp");
        let content = serde_json::to_string_pretty(sessions).map_err(|e| e.to_string())?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&temp_path, content).map_err(|e| e.to_string())?;
        fs::rename(&temp_path, &self.path).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn get_active(&self, key: &SessionKey) -> Result<Option<SessionData>, String> {
        let _guard = self.lock.lock().await;
        let sessions = self.load()?;
        Ok(sessions.get(&key.storage_key()).cloned())
    }

    pub async fn resolve_session(&self, key: &SessionKey, provider: &str, model: &str) -> Result<(SessionData, bool), String> {
        let _guard = self.lock.lock().await;
        let mut sessions = self.load()?;
        let skey = key.storage_key();
        
        let mut resolved_topic_name = None;
        if let Some(ref resolver) = self.resolver {
            if let Some(tid) = key.topic_id {
                resolved_topic_name = resolver(key.chat_id, tid);
            }
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

    pub async fn preserve_session_identity(&self, session: &SessionData) -> Result<SessionData, String> {
        let _guard = self.lock.lock().await;
        let mut sessions = self.load()?;
        let skey = session.session_key().storage_key();
        let mut current = sessions.remove(&skey).unwrap_or_else(|| session.clone());
        current.provider = session.provider.clone();
        current.model = session.model.clone();
        current.language = session.language.clone();
        current.pending_attachments = session.pending_attachments.clone();
        if session.topic_name.is_some() && current.topic_name.is_none() {
            current.topic_name = session.topic_name.clone();
        }
        for (prov, data) in &session.provider_sessions {
            let cur_data = current.provider_sessions.entry(prov.clone()).or_default();
            if !data.session_id.is_empty() { cur_data.session_id = data.session_id.clone(); }
            cur_data.message_count = cur_data.message_count.max(data.message_count);
            cur_data.total_cost_usd = cur_data.total_cost_usd.max(data.total_cost_usd);
            cur_data.total_tokens = cur_data.total_tokens.max(data.total_tokens);
        }
        current.last_active = Utc::now().to_rfc3339();
        sessions.insert(skey, current.clone());
        self.save(&sessions)?;
        Ok(current)
    }

    pub async fn update_session(&self, session: &SessionData, cost_usd: f64, tokens: i64) -> Result<SessionData, String> {
        let mut cur = self.preserve_session_identity(session).await?;
        let _guard = self.lock.lock().await;
        let mut sessions = self.load()?;
        let skey = cur.session_key().storage_key();
        let act_ps = cur.provider_sessions.entry(cur.provider.clone()).or_default();
        act_ps.message_count += 1;
        act_ps.total_cost_usd += cost_usd;
        act_ps.total_tokens += tokens;
        sessions.insert(skey, cur.clone());
        self.save(&sessions)?;
        Ok(cur)
    }

    pub async fn reset_session(&self, k: &SessionKey, provider: &str, model: &str) -> Result<SessionData, String> {
        let _guard = self.lock.lock().await;
        let mut map = self.load()?;
        let skey = k.storage_key();
        let new_s = SessionData::new(k.chat_id, k.transport.clone(), k.topic_id, provider.to_string(), model.to_string());
        map.insert(skey, new_s.clone());
        self.save(&map)?;
        Ok(new_s)
    }

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

    pub async fn sync_session_target(&self, s: &mut SessionData, provider: &str, model: &str) -> Result<(), String> {
        let _guard = self.lock.lock().await;
        let mut map = self.load()?;
        if let Some(cur) = map.get_mut(&s.session_key().storage_key()) {
            let p_chg = cur.provider != provider;
            let m_chg = cur.model != model;
            if p_chg || m_chg {
                if p_chg { cur.provider = provider.to_string(); }
                if m_chg { cur.model = model.to_string(); }
                s.provider = cur.provider.clone();
                s.model = cur.model.clone();
                self.save(&map)?;
            }
        }
        Ok(())
    }

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
            } else {
                map.insert(k, v);
            }
        }
        if chg { self.save(&map)?; }
        Ok(())
    }

    pub fn with_topic_resolver(mut self, resolver: TopicNameResolver) -> Self {
        self.resolver = Some(resolver);
        self
    }

    pub async fn list_active_for_chat(&self, cid: i64) -> Result<Vec<SessionData>, String> {
        let _guard = self.lock.lock().await;
        Ok(self.load()?.into_values().filter(|s| s.chat_id == cid && self.is_fresh(s)).collect())
    }

    pub async fn list_all(&self) -> Result<Vec<SessionData>, String> {
        let _guard = self.lock.lock().await;
        Ok(self.load()?.into_values().collect())
    }
}
