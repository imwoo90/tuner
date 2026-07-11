//! # Observers Wire Module
//!
//! Handles wiring up background observers (heartbeat, cron, background, webhooks) to
//! submit their output results automatically to the `MessageBus`.

use std::sync::{Arc, Mutex};
use async_trait::async_trait;
use super::bus::MessageBus;
use super::envelope::{Envelope, Origin, LockMode, DeliveryMode};
use super::adapters::{
    WebhookResult, InterAgentResult, TaskResult,
    from_heartbeat, from_cron_result, from_background_result, from_webhook_cron_result,
};
use crate::background::models::BackgroundResult;

#[async_trait]
pub trait HeartbeatObserverTrait: Send + Sync {
    async fn set_result_handler(&self, handler: Arc<dyn for<'a> Fn(i64, &'a str, Option<i64>) + Send + Sync>);
}

#[async_trait]
pub trait CronObserverTrait: Send + Sync {
    async fn set_result_handler(&self, handler: Arc<dyn for<'a, 'b, 'c> Fn(&'a str, &'b str, &'c str) + Send + Sync>);
}

#[async_trait]
pub trait WebhookObserverTrait: Send + Sync {
    async fn set_result_handler(&self, handler: Arc<dyn Fn(WebhookResult) + Send + Sync>);
    async fn set_wake_handler(&self, handler: Arc<dyn for<'a> Fn(i64, &'a str) + Send + Sync>);
}

pub struct ObserverManager {
    pub heartbeat: Option<Arc<dyn HeartbeatObserverTrait>>,
    pub cron: Option<Arc<dyn CronObserverTrait>>,
    pub background: Option<crate::background::BackgroundObserver>,
    pub webhook: Option<Arc<dyn WebhookObserverTrait>>,
}

impl ObserverManager {
    /// Wire all present observers to submit their output envelopes to the provided bus.
    pub async fn wire_to_bus(
        &self,
        bus: Arc<MessageBus>,
        wake_handler: Option<Arc<dyn for<'a> Fn(i64, &'a str) + Send + Sync>>,
    ) {
        self.wire_heartbeat(bus.clone()).await;
        self.wire_cron(bus.clone()).await;
        self.wire_webhook(bus.clone(), wake_handler).await;
        self.wire_background(bus).await;
    }

    async fn wire_heartbeat(&self, bus: Arc<MessageBus>) {
        if let Some(ref hb) = self.heartbeat {
            hb.set_result_handler(Arc::new(move |chat_id, alert_text, topic_id| {
                let bus = bus.clone();
                let alert_text = alert_text.to_string();
                tokio::spawn(async move {
                    let mut env = from_heartbeat(chat_id, &alert_text, topic_id, None);
                    bus.submit(&mut env).await;
                });
            })).await;
        }
    }

    async fn wire_cron(&self, bus: Arc<MessageBus>) {
        if let Some(ref cr) = self.cron {
            cr.set_result_handler(Arc::new(move |title, result_text, status| {
                let bus = bus.clone();
                let title = title.to_string();
                let result_text = result_text.to_string();
                let status = status.to_string();
                tokio::spawn(async move {
                    let mut env = from_cron_result(&title, &result_text, &status, None, None, None);
                    bus.submit(&mut env).await;
                });
            })).await;
        }
    }

    async fn wire_webhook(
        &self,
        bus: Arc<MessageBus>,
        wake_handler: Option<Arc<dyn for<'a> Fn(i64, &'a str) + Send + Sync>>,
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

            if let Some(ref wake) = wake_handler {
                wh.set_wake_handler(wake.clone()).await;
            }
        }
    }

    async fn wire_background(&self, bus: Arc<MessageBus>) {
        if let Some(ref bg) = self.background {
            bg.set_result_handler(move |res| {
                let bus = bus.clone();
                tokio::spawn(async move {
                    let mut env = from_background_result(&res);
                    bus.submit(&mut env).await;
                });
            }).await;
        }
    }
}
