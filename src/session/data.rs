//! # Session Data Structure Models
//!
//! ## Overview
//! Defines the [`SessionData`] struct capturing runtime state, provider, models, and history counts
//! for active messaging streams.
//!
//! ## Search Tags
//! #session-data, #model-override, #session-history, #metadata

use std::collections::HashMap;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use crate::session::key::SessionKey;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
/// Represents `ProviderSessionData` (provider session data).
pub struct ProviderSessionData {
    #[serde(default)]
    /// Session id.
    pub session_id: String,
    #[serde(default)]
    /// Past session ids.
    pub past_session_ids: Vec<String>,
    #[serde(default)]
    /// Message count.
    pub message_count: i64,
}

impl Default for SessionData {
    fn default() -> Self {
        let now = default_now_iso();
        Self {
            transport: default_transport(),
            chat_id: 0,
            topic_id: None,
            topic_name: None,
            provider: default_provider(),
            model: default_model(),
            effort: None,
            created_at: now.clone(),
            last_active: now,
            provider_sessions: std::collections::HashMap::new(),
            language: None,
            last_progress_msg_id: None,
            pending_attachments: Vec::new(),
            last_handover_summary: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// Represents `SessionData` (session data).
pub struct SessionData {
    #[serde(default = "default_transport")]
    /// Transport.
    pub transport: String,
    /// Chat id.
    pub chat_id: i64,
    /// Topic id.
    pub topic_id: Option<i64>,
    /// Topic name.
    pub topic_name: Option<String>,
    #[serde(default = "default_provider")]
    /// Provider.
    pub provider: String,
    #[serde(default = "default_model")]
    /// Model.
    pub model: String,
    #[serde(default)]
    /// Effort.
    pub effort: Option<String>,
    #[serde(default = "default_now_iso")]
    /// Created at.
    pub created_at: String,
    #[serde(default = "default_now_iso")]
    /// Last active.
    pub last_active: String,
    #[serde(default)]
    /// Provider sessions.
    pub provider_sessions: HashMap<String, ProviderSessionData>,
    #[serde(default)]
    /// Language.
    pub language: Option<String>,
    #[serde(default)]
    /// Last progress msg id.
    pub last_progress_msg_id: Option<i32>,
    #[serde(default)]
    /// Pending attachments.
    pub pending_attachments: Vec<String>,
    #[serde(default)]
    /// Last handover summary.
    pub last_handover_summary: Option<String>,
}

fn default_transport() -> String { "tg".to_string() }
fn default_provider() -> String { "antigravity".to_string() }
fn default_model() -> String { "gemini-3.8-flash".to_string() }
fn default_now_iso() -> String { Utc::now().to_rfc3339() }

impl SessionData {
    /// Creates a new instance.
    pub fn new(chat_id: i64, transport: String, topic_id: Option<i64>, provider: String, model: String) -> Self {
        let now = default_now_iso();
        Self {
            transport,
            chat_id,
            topic_id,
            topic_name: None,
            provider,
            model,
            effort: None,
            created_at: now.clone(),
            last_active: now,
            provider_sessions: HashMap::new(),
            language: None,
            last_progress_msg_id: None,
            pending_attachments: Vec::new(),
            last_handover_summary: None,
        }
    }

    /// Executes `session_key` (session key).
    pub fn session_key(&self) -> SessionKey {
        SessionKey {
            transport: self.transport.clone(),
            chat_id: self.chat_id,
            topic_id: self.topic_id,
        }
    }

    fn resolve_slot_key(provider: &str) -> &str {
        Self::canonical_provider(provider).unwrap_or_else(|| provider.trim())
    }

    pub(crate) fn fold_provider_aliases(&mut self, canon: &str) -> bool {
        if Self::canonical_provider(canon) != Some(canon) {
            return false;
        }
        let alias_keys: Vec<String> = self
            .provider_sessions
            .keys()
            .filter(|k| k.as_str() != canon && Self::canonical_provider(k) == Some(canon))
            .cloned()
            .collect();
        if alias_keys.is_empty() {
            return false;
        }
        for k in alias_keys {
            if let Some(old_ps) = self.provider_sessions.remove(&k) {
                let dst = self.provider_sessions.entry(canon.to_string()).or_default();
                super::merge_provider_session(dst, &old_ps);
            }
        }
        true
    }

    /// Executes `get_session_id` (get session id).
    pub fn get_session_id(&self, provider: &str) -> String {
        let key = Self::resolve_slot_key(provider);
        if let Some(ps) = self.provider_sessions.get(key)
            && !ps.session_id.is_empty()
        {
            return ps.session_id.clone();
        }
        if Self::canonical_provider(key) == Some(key) {
            for (k, ps) in &self.provider_sessions {
                if Self::canonical_provider(k) == Some(key) && !ps.session_id.is_empty() {
                    return ps.session_id.clone();
                }
            }
        }
        String::new()
    }

    /// Returns the persisted message count for `provider` (across canonical and alias keys).
    pub fn get_message_count(&self, provider: &str) -> i64 {
        let key = Self::resolve_slot_key(provider);
        let is_canon = Self::canonical_provider(key) == Some(key);
        self.provider_sessions
            .iter()
            .filter(|(k, _)| k.as_str() == key || (is_canon && Self::canonical_provider(k) == Some(key)))
            .map(|(_, ps)| ps.message_count)
            .max()
            .unwrap_or(0)
    }

    /// Increments the persisted message count for `provider` under its canonical key.
    pub fn increment_message_count(&mut self, provider: &str) {
        let key = Self::resolve_slot_key(provider).to_string();
        let _ = self.fold_provider_aliases(&key);
        self.provider_sessions.entry(key).or_default().message_count += 1;
    }

    /// Executes `set_session_id` (set session id).
    pub fn set_session_id(&mut self, provider: &str, session_id: &str) {
        let key = Self::resolve_slot_key(provider).to_string();
        let _ = self.fold_provider_aliases(&key);
        let ps = self.provider_sessions.entry(key).or_default();
        if !ps.session_id.is_empty() && ps.session_id != session_id
            && !ps.past_session_ids.contains(&ps.session_id)
        {
            ps.past_session_ids.push(ps.session_id.clone());
        }
        if ps.session_id != session_id {
            ps.message_count = 0;
        }
        ps.session_id = session_id.to_string();
    }

    /// Executes `get_past_session_ids` (get past session ids).
    pub fn get_past_session_ids(&self, provider: &str) -> Vec<String> {
        let key = Self::resolve_slot_key(provider);
        let mut out = self
            .provider_sessions
            .get(key)
            .map(|ps| ps.past_session_ids.clone())
            .unwrap_or_default();
        if Self::canonical_provider(key) == Some(key) {
            for (k, ps) in &self.provider_sessions {
                if k != key && Self::canonical_provider(k) == Some(key) {
                    for id in &ps.past_session_ids {
                        if !out.contains(id) {
                            out.push(id.clone());
                        }
                    }
                }
            }
        }
        out
    }

    /// Executes `clear_provider_session` (clear provider session).
    pub fn clear_provider_session(&mut self, provider: &str) {
        let key = Self::resolve_slot_key(provider).to_string();
        let _ = self.fold_provider_aliases(&key);
        if let Some(ps) = self.provider_sessions.get_mut(&key) {
            if !ps.session_id.is_empty() && !ps.past_session_ids.contains(&ps.session_id) {
                ps.past_session_ids.push(ps.session_id.clone());
            }
            ps.session_id = String::new();
            ps.message_count = 0;
        }
    }
}
