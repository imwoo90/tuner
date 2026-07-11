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

pub struct SessionManager {
    sessions_path: PathBuf,
    idle_timeout_minutes: i64,
    daily_reset_hour: u32,
    daily_reset_enabled: bool,
    user_timezone: String,
    max_session_messages: Option<i64>,
    lock: Mutex<()>,
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
            sessions_path,
            idle_timeout_minutes,
            daily_reset_hour,
            daily_reset_enabled,
            user_timezone,
            max_session_messages,
            lock: Mutex::new(()),
        }
    }

    pub fn is_fresh(&self, session: &SessionData) -> bool {
        super::freshness::is_session_fresh(
            session,
            self.max_session_messages,
            self.idle_timeout_minutes,
            self.daily_reset_enabled,
            self.daily_reset_hour,
            &self.user_timezone,
        )
    }

    pub fn load(&self) -> Result<HashMap<String, SessionData>, String> {
        if !self.sessions_path.exists() {
            return Ok(HashMap::new());
        }
        let content = fs::read_to_string(&self.sessions_path).map_err(|e| e.to_string())?;
        if content.trim().is_empty() {
            return Ok(HashMap::new());
        }
        let raw_map: HashMap<String, serde_json::Value> = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        let mut result = HashMap::new();
        for (k, v) in raw_map {
            let parsed_key = k.parse::<SessionKey>()?;
            let mut sd: SessionData = serde_json::from_value(v).map_err(|e| e.to_string())?;
            if sd.transport.is_empty() {
                sd.transport = parsed_key.transport.clone();
            }
            sd.migrate_legacy_metrics();
            result.insert(parsed_key.storage_key(), sd);
        }
        Ok(result)
    }

    pub fn save(&self, sessions: &HashMap<String, SessionData>) -> Result<(), String> {
        let temp_path = self.sessions_path.with_extension("tmp");
        let content = serde_json::to_string_pretty(sessions).map_err(|e| e.to_string())?;
        if let Some(parent) = self.sessions_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&temp_path, content).map_err(|e| e.to_string())?;
        fs::rename(&temp_path, &self.sessions_path).map_err(|e| e.to_string())?;
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
        
        let mut to_save = false;
        let mut res = None;
        if let Some(existing) = sessions.get_mut(&skey) {
            if self.is_fresh(existing) {
                if existing.provider != provider {
                    existing.provider = provider.to_string();
                    to_save = true;
                }
                if existing.model != model {
                    existing.model = model.to_string();
                    to_save = true;
                }
                let session_id = existing.get_session_id(provider);
                res = Some((existing.clone(), session_id.is_empty()));
            }
        }
        
        if let Some(r) = res {
            if to_save {
                self.save(&sessions)?;
            }
            return Ok(r);
        }

        let new_session = SessionData::new(
            key.chat_id,
            key.transport.clone(),
            key.topic_id,
            provider.to_string(),
            model.to_string(),
        );
        sessions.insert(skey, new_session.clone());
        self.save(&sessions)?;
        Ok((new_session, true))
    }

    pub async fn update_session(&self, session: &SessionData, cost_usd: f64, tokens: i64) -> Result<SessionData, String> {
        let _guard = self.lock.lock().await;
        let mut sessions = self.load()?;
        let skey = session.session_key().storage_key();
        
        let mut current = sessions.remove(&skey).unwrap_or_else(|| session.clone());
        
        // Merge identity fields from incoming
        current.provider = session.provider.clone();
        current.model = session.model.clone();
        if session.topic_name.is_some() && current.topic_name.is_none() {
            current.topic_name = session.topic_name.clone();
        }

        // Merge provider sessions
        for (prov, data) in &session.provider_sessions {
            let cur_data = current.provider_sessions.entry(prov.clone()).or_default();
            if !data.session_id.is_empty() {
                cur_data.session_id = data.session_id.clone();
            }
            cur_data.message_count = cur_data.message_count.max(data.message_count);
            cur_data.total_cost_usd = cur_data.total_cost_usd.max(data.total_cost_usd);
            cur_data.total_tokens = cur_data.total_tokens.max(data.total_tokens);
        }

        // Increment active provider session stats
        let act_ps = current.provider_sessions.entry(current.provider.clone()).or_default();
        act_ps.message_count += 1;
        act_ps.total_cost_usd += cost_usd;
        act_ps.total_tokens += tokens;

        current.last_active = Utc::now().to_rfc3339();
        
        sessions.insert(skey, current.clone());
        self.save(&sessions)?;
        Ok(current)
    }

    pub async fn reset_session(&self, key: &SessionKey, provider: &str, model: &str) -> Result<SessionData, String> {
        let _guard = self.lock.lock().await;
        let mut sessions = self.load()?;
        let skey = key.storage_key();
        let new_session = SessionData::new(
            key.chat_id,
            key.transport.clone(),
            key.topic_id,
            provider.to_string(),
            model.to_string(),
        );
        sessions.insert(skey, new_session.clone());
        self.save(&sessions)?;
        Ok(new_session)
    }

    pub async fn reset_provider_session(
        &self,
        key: &SessionKey,
        provider: &str,
        model: &str,
    ) -> Result<SessionData, String> {
        let _guard = self.lock.lock().await;
        let mut sessions = self.load()?;
        let skey = key.storage_key();
        
        let current = match sessions.get_mut(&skey) {
            Some(existing) => {
                existing.clear_provider_session(provider);
                existing.provider = provider.to_string();
                existing.model = model.to_string();
                existing.last_active = Utc::now().to_rfc3339();
                existing.clone()
            }
            None => {
                SessionData::new(
                    key.chat_id,
                    key.transport.clone(),
                    key.topic_id,
                    provider.to_string(),
                    model.to_string(),
                )
            }
        };

        if sessions.get(&skey).is_none() {
            sessions.insert(skey.clone(), current.clone());
        }
        self.save(&sessions)?;
        Ok(current)
    }


    pub async fn sync_session_target(&self, session: &mut SessionData, provider: &str, model: &str) -> Result<(), String> {
        let _guard = self.lock.lock().await;
        let mut sessions = self.load()?;
        let skey = session.session_key().storage_key();
        
        let mut changed = false;
        let mut updated_prov = None;
        let mut updated_model = None;
        if let Some(current) = sessions.get_mut(&skey) {
            if current.provider != provider {
                current.provider = provider.to_string();
                changed = true;
            }
            if current.model != model {
                current.model = model.to_string();
                changed = true;
            }
            if changed {
                updated_prov = Some(current.provider.clone());
                updated_model = Some(current.model.clone());
            }
        }
        
        if changed {
            self.save(&sessions)?;
            if let Some(p) = updated_prov {
                session.provider = p;
            }
            if let Some(m) = updated_model {
                session.model = m;
            }
        }
        Ok(())
    }

    pub async fn migrate_chat_id(&self, old_chat_id: i64, new_chat_id: i64) -> Result<(), String> {
        let _guard = self.lock.lock().await;
        let sessions = self.load()?;
        let mut migrated = HashMap::new();
        let mut changed = false;

        for (k, mut v) in sessions {
            if v.chat_id == old_chat_id {
                v.chat_id = new_chat_id;
                let parsed = k.parse::<SessionKey>()?;
                let new_key = SessionKey {
                    transport: parsed.transport,
                    chat_id: new_chat_id,
                    topic_id: parsed.topic_id,
                }.storage_key();
                migrated.insert(new_key, v);
                changed = true;
            } else {
                migrated.insert(k, v);
            }
        }

        if changed {
            self.save(&migrated)?;
        }
        Ok(())
    }
}
