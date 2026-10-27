//! # Adapters Module
//!
//! ## Overview
//! Provides conversion functions to map domain events and results
//! (Cron and Heartbeat triggers) into unified `Envelope` models.
//!
//! ## Search Tags
//! #adapters

use super::envelope::{Envelope, Origin, DeliveryMode};

/// Executes `from_cron_result` (from cron result).
pub fn from_cron_result(
    title: &str,
    result_text: &str,
    status: &str,
    chat_id: Option<i64>,
    topic_id: Option<i64>,
    transport: Option<&str>,
) -> Envelope {
    let mut env = Envelope::new(Origin::Cron, chat_id.unwrap_or(0));
    env.topic_id = topic_id;
    env.result_text = result_text.to_string();
    env.status = status.to_string();
    env.is_error = status.to_lowercase().contains("error");
    if chat_id.is_some() {
        env.delivery = DeliveryMode::Unicast;
    } else {
        env.delivery = DeliveryMode::Broadcast;
    }
    if let Some(t) = transport {
        env.transport = t.to_string();
    }
    env.metadata.insert("title".to_string(), title.to_string());
    env
}

/// Executes `from_heartbeat` (from heartbeat).
pub fn from_heartbeat(
    chat_id: i64,
    alert_text: &str,
    topic_id: Option<i64>,
    transport: Option<&str>,
) -> Envelope {
    let mut env = Envelope::new(Origin::Heartbeat, chat_id);
    env.topic_id = topic_id;
    env.result_text = alert_text.to_string();
    env.status = "success".to_string();
    if let Some(t) = transport {
        env.transport = t.to_string();
    }
    env
}
