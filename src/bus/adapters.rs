//! # Adapters Module
//!
//! ## Overview
//! Provides conversion functions to map domain events and results
//! (Cron, Heartbeat, and Webhook triggers) into unified `Envelope` models.
//!
//! ## Search Tags
//! #adapters

use super::envelope::{Envelope, Origin, LockMode, DeliveryMode};

#[derive(Debug, Clone)]
/// Represents `WebhookResult` (webhook result).
pub struct WebhookResult {
    /// Hook id.
    pub hook_id: String,
    /// Hook title.
    pub hook_title: String,
    /// Result text.
    pub result_text: String,
    /// Status.
    pub status: String,
}

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

/// Executes `from_webhook_cron_result` (from webhook cron result).
pub fn from_webhook_cron_result(result: &WebhookResult) -> Envelope {
    let mut env = Envelope::new(Origin::WebhookCron, 0);
    env.result_text = result.result_text.clone();
    env.status = result.status.clone();
    env.is_error = result.status.to_lowercase().contains("error");
    env.delivery = DeliveryMode::Broadcast;
    env.metadata.insert("hook_title".to_string(), result.hook_title.clone());
    env.metadata.insert("hook_id".to_string(), result.hook_id.clone());
    env
}

/// Executes `from_webhook_wake` (from webhook wake).
pub fn from_webhook_wake(chat_id: i64, prompt: &str) -> Envelope {
    let mut env = Envelope::new(Origin::WebhookWake, chat_id);
    env.prompt = prompt.to_string();
    env.delivery = DeliveryMode::Unicast;
    env.lock_mode = LockMode::Required;
    env.needs_injection = true;
    env
}
