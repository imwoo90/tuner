//! # Envelope Module
//!
//! ## Overview
//! Defines the `Envelope` structure and its associated enums (`Origin`, `DeliveryMode`, `LockMode`).
//! Envelopes encapsulate message data, routing metadata, and execution status for delivery across the message bus.
//!
//! ## Search Tags
//! #envelope

use std::collections::HashMap;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Enumerates `Origin` (origin) variants.
pub enum Origin {
    /// `Background` (background) variant.
    Background,
    /// `Cron` (cron) variant.
    Cron,
    /// `WebhookWake` (webhook wake) variant.
    WebhookWake,
    /// `WebhookCron` (webhook cron) variant.
    WebhookCron,
    /// `Heartbeat` (heartbeat) variant.
    Heartbeat,
    /// `Interagent` (interagent) variant.
    Interagent,
    /// `TaskResult` (task result) variant.
    TaskResult,
    /// `TaskQuestion` (task question) variant.
    TaskQuestion,
    /// `User` (user) variant.
    User,
    /// `Api` (api) variant.
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
/// Enumerates `DeliveryMode` (delivery mode) variants.
pub enum DeliveryMode {
    /// `Unicast` (unicast) variant.
    Unicast,
    /// `Broadcast` (broadcast) variant.
    Broadcast,
}

impl DeliveryMode {
    /// Executes `as_str` (as str).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unicast => "unicast",
            Self::Broadcast => "broadcast",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Enumerates `LockMode` (lock mode) variants.
pub enum LockMode {
    /// `Required` (required) variant.
    Required,
    /// `None` (none) variant.
    None,
}

impl LockMode {
    /// Executes `as_str` (as str).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Represents `Envelope` (envelope).
pub struct Envelope {
    /// Origin.
    pub origin: Origin,
    /// Chat id.
    pub chat_id: i64,
    /// Topic id.
    pub topic_id: Option<i64>,
    /// Prompt.
    pub prompt: String,
    /// Prompt preview.
    pub prompt_preview: String,
    /// Result text.
    pub result_text: String,
    /// Status.
    pub status: String,
    /// Is error.
    pub is_error: bool,
    /// Delivery.
    pub delivery: DeliveryMode,
    /// Lock mode.
    pub lock_mode: LockMode,
    /// Needs injection.
    pub needs_injection: bool,
    /// Metadata.
    pub metadata: HashMap<String, String>,
    /// Reply to message id.
    pub reply_to_message_id: Option<i64>,
    /// Thread id.
    pub thread_id: Option<i64>,
    /// Envelope id.
    pub envelope_id: String,
    /// Elapsed seconds.
    pub elapsed_seconds: f64,
    /// Provider.
    pub provider: String,
    /// Model.
    pub model: String,
    /// Session name.
    pub session_name: String,
    /// Session id.
    pub session_id: String,
    /// Created at.
    pub created_at: u64,
    /// Transport.
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
