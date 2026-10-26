//! # Observers Wire Module
//!
//! ## Overview
//! Handles wiring up webhook observers to submit their output results and wake
//! prompts automatically to the `MessageBus`.
//!
//! ## Search Tags
//! #observers-wire

use std::sync::Arc;
use async_trait::async_trait;
use super::bus::MessageBus;
use super::adapters::{from_webhook_cron_result, from_webhook_wake, WebhookResult};

#[async_trait]
/// Defines the `WebhookObserverTrait` (webhook observer trait) interface.
pub trait WebhookObserverTrait: Send + Sync {
    /// Executes `set_result_handler` (set result handler).
    async fn set_result_handler(&self, handler: Arc<dyn Fn(WebhookResult) + Send + Sync>);
    /// Executes `set_wake_handler` (set wake handler).
    async fn set_wake_handler(&self, handler: Arc<dyn for<'a> Fn(i64, &'a str) + Send + Sync>);
}

/// Callback type invoked when a wake webhook is triggered.
pub type WakeHandlerCallback = Arc<dyn for<'a> Fn(i64, &'a str) + Send + Sync>;

/// Represents `ObserverManager` (observer manager).
pub struct ObserverManager {
    /// Webhook.
    pub webhook: Option<Arc<dyn WebhookObserverTrait>>,
}

impl ObserverManager {
    /// Wire present observers to submit their output envelopes to the provided bus.
    pub async fn wire_to_bus(
        &self,
        bus: Arc<MessageBus>,
        wake_handler: Option<WakeHandlerCallback>,
    ) {
        self.wire_webhook(bus, wake_handler).await;
    }

    async fn wire_webhook(
        &self,
        bus: Arc<MessageBus>,
        wake_handler: Option<WakeHandlerCallback>,
    ) {
        if let Some(ref wh) = self.webhook {
            let bus_clone = bus.clone();
            wh.set_result_handler(Arc::new(move |res| {
                let bus = bus_clone.clone();
                if res.result_text.starts_with("wake") {
                    return;
                }
                tokio::spawn(async move {
                    let mut env = from_webhook_cron_result(&res);
                    bus.submit(&mut env).await;
                });
            })).await;

            if let Some(wake) = wake_handler {
                wh.set_wake_handler(wake).await;
            } else {
                let bus_wake = bus.clone();
                wh.set_wake_handler(Arc::new(move |chat_id, prompt| {
                    let bus = bus_wake.clone();
                    let prompt_owned = prompt.to_string();
                    tokio::spawn(async move {
                        let mut env = from_webhook_wake(chat_id, &prompt_owned);
                        bus.submit(&mut env).await;
                    });
                })).await;
            }
        }
    }
}
