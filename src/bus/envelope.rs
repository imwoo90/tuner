//! # Envelope Module
//!
//! Defines the `Envelope` structure and its associated enums (`Origin`, `DeliveryMode`, `LockMode`).
//! Envelopes encapsulate message data, routing metadata, and execution status for delivery across the message bus.

//! 
//! ## Search Tags
//! #envelope

use std::collections::HashMap;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Background,
    Cron,
    WebhookWake,
    WebhookCron,
    Heartbeat,
    Interagent,
    TaskResult,
    TaskQuestion,
    User,
    Api,
}

impl Origin {
    /// Return the string value as defined in the Python implementation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Cron => "cron",
            Self::WebhookWake => "webhook_wake",
            Self::WebhookCron => "webhook_cron",
            Self::Heartbeat => "heartbeat",
            Self::Interagent => "interagent",
            Self::TaskResult => "task_result",
            Self::TaskQuestion => "task_question",
            Self::User => "user",
            Self::Api => "api",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    Unicast,
    Broadcast,
}

impl DeliveryMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unicast => "unicast",
            Self::Broadcast => "broadcast",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockMode {
    Required,
    None,
}

impl LockMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub origin: Origin,
    pub chat_id: i64,
    pub topic_id: Option<i64>,
    pub prompt: String,
    pub prompt_preview: String,
    pub result_text: String,
    pub status: String,
    pub is_error: bool,
    pub delivery: DeliveryMode,
    pub lock_mode: LockMode,
    pub needs_injection: bool,
    pub metadata: HashMap<String, String>,
    pub reply_to_message_id: Option<i64>,
    pub thread_id: Option<i64>,
    pub envelope_id: String,
    pub elapsed_seconds: f64,
    pub provider: String,
    pub model: String,
    pub session_name: String,
    pub session_id: String,
    pub created_at: u64,
    pub transport: String,
}

impl Envelope {
    /// Create a new envelope with default values.
    pub fn new(origin: Origin, chat_id: i64) -> Self {
        let created_at = chrono::Utc::now().timestamp() as u64;
        Self {
            origin,
            chat_id,
            topic_id: None,
            prompt: String::new(),
            prompt_preview: String::new(),
            result_text: String::new(),
            status: String::new(),
            is_error: false,
            delivery: DeliveryMode::Unicast,
            lock_mode: LockMode::None,
            needs_injection: false,
            metadata: HashMap::new(),
            reply_to_message_id: None,
            thread_id: None,
            envelope_id: String::new(),
            elapsed_seconds: 0.0,
            provider: String::new(),
            model: String::new(),
            session_name: String::new(),
            session_id: String::new(),
            created_at,
            transport: "tg".to_string(),
        }
    }

    /// Return the lock key tuple.
    pub fn lock_key(&self) -> (i64, Option<i64>) {
        (self.chat_id, self.topic_id)
    }
}
