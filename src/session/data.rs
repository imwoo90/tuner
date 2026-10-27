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

    /// Executes `get_session_id` (get session id).
    pub fn get_session_id(&self, provider: &str) -> String {
        self.provider_sessions.get(provider)
            .map(|ps| ps.session_id.clone())
            .unwrap_or_default()
    }

    /// Executes `set_session_id` (set session id).
    pub fn set_session_id(&mut self, provider: &str, session_id: &str) {
        let ps = self.provider_sessions.entry(provider.to_string()).or_default();
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
        self.provider_sessions.get(provider)
            .map(|ps| ps.past_session_ids.clone())
            .unwrap_or_default()
    }

    /// Executes `clear_provider_session` (clear provider session).
    pub fn clear_provider_session(&mut self, provider: &str) {
        if let Some(ps) = self.provider_sessions.get_mut(provider) {
            if !ps.session_id.is_empty() && !ps.past_session_ids.contains(&ps.session_id) {
                ps.past_session_ids.push(ps.session_id.clone());
            }
            ps.session_id = String::new();
            ps.message_count = 0;
        }
    }
}
