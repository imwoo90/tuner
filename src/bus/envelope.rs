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
    /// `Cron` (cron) variant.
    Cron,
    /// `Heartbeat` (heartbeat) variant.
    Heartbeat,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Enumerates `LockMode` (lock mode) variants.
pub enum LockMode {
    /// `Required` (required) variant.
    Required,
    /// `None` (none) variant.
    None,
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
    /// Metadata.
    pub metadata: HashMap<String, String>,
    /// Envelope id.
    pub envelope_id: String,
    /// Transport.
    pub transport: String,
}

impl Envelope {
    /// Create a new envelope with default values.
    pub fn new(origin: Origin, chat_id: i64) -> Self {
        Self {
            origin,
            chat_id,
            topic_id: None,
            result_text: String::new(),
            status: String::new(),
            is_error: false,
            delivery: DeliveryMode::Unicast,
            lock_mode: LockMode::None,
            metadata: HashMap::new(),
            envelope_id: String::new(),
            transport: "tg".to_string(),
        }
    }

    /// Return the lock key tuple.
    pub fn lock_key(&self) -> (i64, Option<i64>) {
        (self.chat_id, self.topic_id)
    }
}
