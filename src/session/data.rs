//! # Session Data Structure Models
//!
//! ## Overview
//! Defines the [`SessionData`] struct capturing runtime state, provider, models, and history counts
//! for active messaging streams.
//!
//! ## Collaboration Graph
//! - Persisted by [`SessionManager`](super::manager::SessionManager).
//! - Loaded/updated dynamically as CLI streams write output chunks.
//!
//! ## Search Tags
//! #session-data, #model-override, #session-history, #metadata

use std::collections::HashMap;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use crate::session::key::SessionKey;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderSessionData {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub message_count: i64,
    #[serde(default)]
    pub total_cost_usd: f64,
    #[serde(default)]
    pub total_tokens: i64,
}

impl Default for ProviderSessionData {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            message_count: 0,
            total_cost_usd: 0.0,
            total_tokens: 0,
        }
    }
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
            session_id: None,
            message_count: None,
            total_cost_usd: None,
            total_tokens: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionData {
    #[serde(default = "default_transport")]
    pub transport: String,
    pub chat_id: i64,
    pub topic_id: Option<i64>,
    pub topic_name: Option<String>,
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default = "default_now_iso")]
    pub created_at: String,
    #[serde(default = "default_now_iso")]
    pub last_active: String,
    #[serde(default)]
    pub provider_sessions: HashMap<String, ProviderSessionData>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub last_progress_msg_id: Option<i32>,
    #[serde(default)]
    pub pending_attachments: Vec<String>,

    // Legacy fields used for migration
    #[serde(skip_serializing, default)]
    pub session_id: Option<String>,
    #[serde(skip_serializing, default)]
    pub message_count: Option<i64>,
    #[serde(skip_serializing, default)]
    pub total_cost_usd: Option<f64>,
    #[serde(skip_serializing, default)]
    pub total_tokens: Option<i64>,
}

fn default_transport() -> String { "tg".to_string() }
fn default_provider() -> String { "claude".to_string() }
fn default_model() -> String { "opus".to_string() }
fn default_now_iso() -> String { Utc::now().to_rfc3339() }

impl SessionData {
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
            session_id: None,
            message_count: None,
            total_cost_usd: None,
            total_tokens: None,
        }
    }

    pub fn session_key(&self) -> SessionKey {
        SessionKey {
            transport: self.transport.clone(),
            chat_id: self.chat_id,
            topic_id: self.topic_id,
        }
    }

    pub fn get_session_id(&self, provider: &str) -> String {
        self.provider_sessions.get(provider)
            .map(|ps| ps.session_id.clone())
            .unwrap_or_default()
    }

    pub fn set_session_id(&mut self, provider: &str, session_id: &str) {
        let ps = self.provider_sessions.entry(provider.to_string()).or_default();
        ps.session_id = session_id.to_string();
    }

    pub fn migrate_legacy_metrics(&mut self) {
        let has_legacy = self.session_id.is_some()
            || self.message_count.is_some()
            || self.total_cost_usd.is_some()
            || self.total_tokens.is_some();

        if has_legacy && !self.provider.is_empty() {
            let ps = self.provider_sessions.entry(self.provider.clone()).or_default();
            if let Some(ref sid) = self.session_id {
                ps.session_id = sid.clone();
            }
            if let Some(cnt) = self.message_count {
                ps.message_count = cnt;
            }
            if let Some(cost) = self.total_cost_usd {
                ps.total_cost_usd = cost;
            }
            if let Some(tok) = self.total_tokens {
                ps.total_tokens = tok;
            }
        }
    }

    pub fn clear_provider_session(&mut self, provider: &str) {
        if let Some(ps) = self.provider_sessions.get_mut(provider) {
            ps.session_id = String::new();
        }
    }
}
