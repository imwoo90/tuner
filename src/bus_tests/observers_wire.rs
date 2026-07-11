use crate::bus::envelope::{Envelope, Origin, LockMode, DeliveryMode};
use crate::bus::bus::{MessageBus, Transport};
use crate::bus::observers_wire::{
    ObserverManager, HeartbeatObserverTrait, CronObserverTrait, WebhookObserverTrait,
};
use crate::bus::adapters::WebhookResult;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use async_trait::async_trait;

// --- Mock Implementations for Tests ---

struct MockHeartbeatObserver {
    handler_set: Arc<AtomicBool>,
    handler: Mutex<Option<Arc<dyn for<'a> Fn(i64, &'a str, Option<i64>) + Send + Sync>>>,
}
#[async_trait]
impl HeartbeatObserverTrait for MockHeartbeatObserver {
    async fn set_result_handler(&self, handler: Arc<dyn for<'a> Fn(i64, &'a str, Option<i64>) + Send + Sync>) {
        self.handler_set.store(true, Ordering::SeqCst);
        *self.handler.lock().unwrap() = Some(handler);
    }
}

struct MockCronObserver {
    handler_set: Arc<AtomicBool>,
    handler: Mutex<Option<Arc<dyn for<'a, 'b, 'c> Fn(&'a str, &'b str, &'c str) + Send + Sync>>>,
}
#[async_trait]
impl CronObserverTrait for MockCronObserver {
    async fn set_result_handler(&self, handler: Arc<dyn for<'a, 'b, 'c> Fn(&'a str, &'b str, &'c str) + Send + Sync>) {
        self.handler_set.store(true, Ordering::SeqCst);
        *self.handler.lock().unwrap() = Some(handler);
    }
}

struct MockWebhookObserver {
    handler_set: Arc<AtomicBool>,
    wake_handler_set: Arc<AtomicBool>,
    handler: Mutex<Option<Arc<dyn Fn(WebhookResult) + Send + Sync>>>,
    wake_handler: Mutex<Option<Arc<dyn for<'a> Fn(i64, &'a str) + Send + Sync>>>,
}
#[async_trait]
impl WebhookObserverTrait for MockWebhookObserver {
    async fn set_result_handler(&self, handler: Arc<dyn Fn(WebhookResult) + Send + Sync>) {
        self.handler_set.store(true, Ordering::SeqCst);
        *self.handler.lock().unwrap() = Some(handler);
    }
    async fn set_wake_handler(&self, handler: Arc<dyn for<'a> Fn(i64, &'a str) + Send + Sync>) {
        self.wake_handler_set.store(true, Ordering::SeqCst);
        *self.wake_handler.lock().unwrap() = Some(handler);
    }
}

struct MockTransport {
    deliver_called: Arc<AtomicBool>,
    deliver_broadcast_called: Arc<AtomicBool>,
    last_envelope: Mutex<Option<Envelope>>,
}
#[async_trait]
impl Transport for MockTransport {
    fn transport_name(&self) -> &str { "tg" }
    async fn deliver(&self, env: &Envelope) -> Result<(), String> {
        self.deliver_called.store(true, Ordering::SeqCst);
        *self.last_envelope.lock().unwrap() = Some(env.clone());
        Ok(())
    }
    async fn deliver_broadcast(&self, env: &Envelope) -> Result<(), String> {
        self.deliver_broadcast_called.store(true, Ordering::SeqCst);
        *self.last_envelope.lock().unwrap() = Some(env.clone());
        Ok(())
    }
}

// --- Unit Tests for Wiring ---

#[tokio::test]
async fn test_heartbeat_handler_wired() {
    let heartbeat = Arc::new(MockHeartbeatObserver {
        handler_set: Arc::new(AtomicBool::new(false)),
        handler: Mutex::new(None),
    });
    let mgr = ObserverManager {
        heartbeat: Some(heartbeat.clone()),
        cron: None,
        background: None,
        webhook: None,
    };

    let bus = Arc::new(MessageBus::new());
    mgr.wire_to_bus(bus, None).await;

    assert!(heartbeat.handler_set.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_cron_handler_wired_when_present() {
    let cron = Arc::new(MockCronObserver {
        handler_set: Arc::new(AtomicBool::new(false)),
        handler: Mutex::new(None),
    });
    let mgr = ObserverManager {
        heartbeat: None,
        cron: Some(cron.clone()),
        background: None,
        webhook: None,
    };

    let bus = Arc::new(MessageBus::new());
    mgr.wire_to_bus(bus, None).await;

    assert!(cron.handler_set.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_cron_handler_skipped_when_none() {
    let mgr = ObserverManager {
        heartbeat: None,
        cron: None,
        background: None,
        webhook: None,
    };
    let bus = Arc::new(MessageBus::new());
    mgr.wire_to_bus(bus, None).await; // No panic/error
}

#[tokio::test]
async fn test_webhook_handler_wired_when_present() {
    let webhook = Arc::new(MockWebhookObserver {
        handler_set: Arc::new(AtomicBool::new(false)),
        wake_handler_set: Arc::new(AtomicBool::new(false)),
        handler: Mutex::new(None),
        wake_handler: Mutex::new(None),
    });
    let mgr = ObserverManager {
        heartbeat: None,
        cron: None,
        background: None,
        webhook: Some(webhook.clone()),
    };

    let bus = Arc::new(MessageBus::new());
    mgr.wire_to_bus(bus, None).await;

    assert!(webhook.handler_set.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_wake_handler_passed_to_webhook() {
    let webhook = Arc::new(MockWebhookObserver {
        handler_set: Arc::new(AtomicBool::new(false)),
        wake_handler_set: Arc::new(AtomicBool::new(false)),
        handler: Mutex::new(None),
        wake_handler: Mutex::new(None),
    });
    let mgr = ObserverManager {
        heartbeat: None,
        cron: None,
        background: None,
        webhook: Some(webhook.clone()),
    };

    let bus = Arc::new(MessageBus::new());
    let wake = Arc::new(|_chat_id, _prompt: &str| {});
    mgr.wire_to_bus(bus, Some(wake)).await;

    assert!(webhook.wake_handler_set.load(Ordering::SeqCst));
}

// --- Webhook Callback Mode Filtering Tests ---

#[tokio::test]
async fn test_webhook_callback_skips_wake_mode() {
    let webhook = Arc::new(MockWebhookObserver {
        handler_set: Arc::new(AtomicBool::new(false)),
        wake_handler_set: Arc::new(AtomicBool::new(false)),
        handler: Mutex::new(None),
        wake_handler: Mutex::new(None),
    });
    let mgr = ObserverManager {
        heartbeat: None,
        cron: None,
        background: None,
        webhook: Some(webhook.clone()),
    };

    let bus = Arc::new(MessageBus::new());
    let transport = Arc::new(MockTransport {
        deliver_called: Arc::new(AtomicBool::new(false)),
        deliver_broadcast_called: Arc::new(AtomicBool::new(false)),
        last_envelope: Mutex::new(None),
    });
    bus.register_transport(transport.clone());
    mgr.wire_to_bus(bus, None).await;

    let handler = webhook.handler.lock().unwrap().clone().unwrap();
    let result = WebhookResult {
        hook_id: "h1".to_string(),
        hook_title: "Test".to_string(),
        status: "success".to_string(),
        result_text: "wake".to_string(),
    };
    handler(result);

    assert!(!transport.deliver_broadcast_called.load(Ordering::SeqCst));
}

// --- Integration tests showing wiring callback reaches bus ---

#[tokio::test]
async fn test_heartbeat_callback_submits_to_bus() {
    let heartbeat = Arc::new(MockHeartbeatObserver {
        handler_set: Arc::new(AtomicBool::new(false)),
        handler: Mutex::new(None),
    });
    let mgr = ObserverManager {
        heartbeat: Some(heartbeat.clone()),
        cron: None,
        background: None,
        webhook: None,
    };

    let bus = Arc::new(MessageBus::new());
    let transport = Arc::new(MockTransport {
        deliver_called: Arc::new(AtomicBool::new(false)),
        deliver_broadcast_called: Arc::new(AtomicBool::new(false)),
        last_envelope: Mutex::new(None),
    });
    bus.register_transport(transport.clone());
    mgr.wire_to_bus(bus, None).await;

    let handler = heartbeat.handler.lock().unwrap().clone().unwrap();
    handler(99, "Alert text", None);

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    assert!(transport.deliver_called.load(Ordering::SeqCst));
    let env = transport.last_envelope.lock().unwrap().clone().unwrap();
    assert_eq!(env.origin, Origin::Heartbeat);
    assert_eq!(env.chat_id, 99);
    assert!(env.topic_id.is_none());
    assert_eq!(env.result_text, "Alert text");
}
