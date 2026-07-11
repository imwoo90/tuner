use crate::bus::envelope::{Envelope, Origin, LockMode, DeliveryMode};
use crate::bus::lock_pool::LockPool;
use crate::bus::bus::{MessageBus, Transport, PromptInjector, BusHook};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;
use async_trait::async_trait;

// Helper to build test envelopes
fn test_envelope(delivery: DeliveryMode, transport: &str) -> Envelope {
    let mut env = Envelope::new(Origin::Cron, 1);
    env.delivery = delivery;
    env.transport = transport.to_string();
    env
}

// Mock Transport
struct MockTransport {
    name: String,
    deliver_calls: Arc<Mutex<Vec<Envelope>>>,
    deliver_broadcast_calls: Arc<Mutex<Vec<Envelope>>>,
    should_fail: bool,
}

impl MockTransport {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            deliver_calls: Arc::new(Mutex::new(Vec::new())),
            deliver_broadcast_calls: Arc::new(Mutex::new(Vec::new())),
            should_fail: false,
        }
    }
}

#[async_trait]
impl Transport for MockTransport {
    fn transport_name(&self) -> &str {
        &self.name
    }
    async fn deliver(&self, envelope: &Envelope) -> Result<(), String> {
        if self.should_fail {
            return Err("Network error".to_string());
        }
        self.deliver_calls.lock().unwrap().push(envelope.clone());
        Ok(())
    }
    async fn deliver_broadcast(&self, envelope: &Envelope) -> Result<(), String> {
        if self.should_fail {
            return Err("Fallback crash".to_string());
        }
        self.deliver_broadcast_calls.lock().unwrap().push(envelope.clone());
        Ok(())
    }
}

// Mock Injector
struct MockInjector {
    response: String,
    calls: Arc<Mutex<Vec<(String, i64, String, Option<i64>, String)>>>,
    should_fail: bool,
}

impl MockInjector {
    fn new(response: &str) -> Self {
        Self {
            response: response.to_string(),
            calls: Arc::new(Mutex::new(Vec::new())),
            should_fail: false,
        }
    }
}

#[async_trait]
impl PromptInjector for MockInjector {
    async fn inject_prompt(
        &self,
        prompt: &str,
        chat_id: i64,
        label: &str,
        topic_id: Option<i64>,
        transport: &str,
    ) -> Result<String, String> {
        if self.should_fail {
            return Err("CLI crash".to_string());
        }
        self.calls.lock().unwrap().push((
            prompt.to_string(),
            chat_id,
            label.to_string(),
            topic_id,
            transport.to_string(),
        ));
        Ok(self.response.clone())
    }
}

#[tokio::test]
async fn test_submit_assigns_envelope_id() {
    let bus = MessageBus::new();
    let mut env = test_envelope(DeliveryMode::Unicast, "tg");
    assert_eq!(env.envelope_id, "");
    bus.submit(&mut env).await;
    assert_ne!(env.envelope_id, "");
}

#[tokio::test]
async fn test_submit_preserves_existing_id() {
    let bus = MessageBus::new();
    let mut env = test_envelope(DeliveryMode::Unicast, "tg");
    env.envelope_id = "custom-id".to_string();
    bus.submit(&mut env).await;
    assert_eq!(env.envelope_id, "custom-id");
}

#[tokio::test]
async fn test_unicast_calls_deliver() {
    let bus = MessageBus::new();
    let transport = Arc::new(MockTransport::new("tg"));
    bus.register_transport(transport.clone());

    let mut env = test_envelope(DeliveryMode::Unicast, "tg");
    bus.submit(&mut env).await;

    assert_eq!(transport.deliver_calls.lock().unwrap().len(), 1);
    assert_eq!(transport.deliver_broadcast_calls.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn test_broadcast_calls_deliver_broadcast() {
    let bus = MessageBus::new();
    let transport = Arc::new(MockTransport::new("tg"));
    bus.register_transport(transport.clone());

    let mut env = test_envelope(DeliveryMode::Broadcast, "tg");
    bus.submit(&mut env).await;

    assert_eq!(transport.deliver_broadcast_calls.lock().unwrap().len(), 1);
    assert_eq!(transport.deliver_calls.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn test_multiple_transports() {
    let bus = MessageBus::new();
    let t1 = Arc::new(MockTransport::new("tg"));
    let t2 = Arc::new(MockTransport::new("mx"));
    bus.register_transport(t1.clone());
    bus.register_transport(t2.clone());

    let mut env = test_envelope(DeliveryMode::Broadcast, "");
    bus.submit(&mut env).await;

    assert_eq!(t1.deliver_broadcast_calls.lock().unwrap().len(), 1);
    assert_eq!(t2.deliver_broadcast_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_lock_required_acquires_lock() {
    let pool = Arc::new(LockPool::new(10));
    let bus = MessageBus::with_lock_pool(pool.clone());

    let lock_acquired_inside = Arc::new(Mutex::new(false));
    let pool_clone = pool.clone();
    let lock_acquired_inside_clone = lock_acquired_inside.clone();

    struct CheckingTransport {
        pool: Arc<LockPool>,
        acquired: Arc<Mutex<bool>>,
    }
    #[async_trait]
    impl Transport for CheckingTransport {
        fn transport_name(&self) -> &str { "tg" }
        async fn deliver(&self, env: &Envelope) -> Result<(), String> {
            let mut guard = self.acquired.lock().unwrap();
            *guard = self.pool.is_locked(env.lock_key());
            Ok(())
        }
        async fn deliver_broadcast(&self, _: &Envelope) -> Result<(), String> { Ok(()) }
    }

    let transport = Arc::new(CheckingTransport { pool: pool_clone, acquired: lock_acquired_inside_clone });
    bus.register_transport(transport);

    let mut env = Envelope::new(Origin::Cron, 42);
    env.lock_mode = LockMode::Required;
    env.transport = "tg".to_string();

    bus.submit(&mut env).await;

    assert!(*lock_acquired_inside.lock().unwrap());
    assert!(!pool.is_locked((42, None))); // Released after submission completes
}

#[tokio::test]
async fn test_injection_updates_result_text() {
    let bus = MessageBus::new();
    let transport = Arc::new(MockTransport::new("tg"));
    bus.register_transport(transport);

    let injector = Arc::new(MockInjector::new("Injected response"));
    bus.set_injector(injector.clone());

    let mut env = test_envelope(DeliveryMode::Unicast, "tg");
    env.needs_injection = true;
    env.prompt = "Injected prompt".to_string();
    env.lock_mode = LockMode::Required;

    bus.submit(&mut env).await;

    assert_eq!(env.result_text, "Injected response");
    assert_eq!(injector.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_injection_failure_sets_error() {
    let bus = MessageBus::new();
    let transport = Arc::new(MockTransport::new("tg"));
    bus.register_transport(transport);

    let mut injector = MockInjector::new("");
    injector.should_fail = true;
    bus.set_injector(Arc::new(injector));

    let mut env = test_envelope(DeliveryMode::Unicast, "tg");
    env.needs_injection = true;
    env.prompt = "test".to_string();

    bus.submit(&mut env).await;

    assert!(env.is_error);
    assert!(env.result_text.contains("CLI crash"));
}

#[tokio::test]
async fn test_pre_deliver_hook_called() {
    let bus = MessageBus::new();
    let transport = Arc::new(MockTransport::new("tg"));
    bus.register_transport(transport);

    let called = Arc::new(Mutex::new(false));
    let called_clone = called.clone();
    bus.set_pre_deliver_hook(Arc::new(move |_| {
        let mut guard = called_clone.lock().unwrap();
        *guard = true;
    }));

    let mut env = test_envelope(DeliveryMode::Unicast, "tg");
    bus.submit(&mut env).await;

    assert!(*called.lock().unwrap());
}

#[tokio::test]
async fn test_deliver_routes_to_matching_transport() {
    let bus = MessageBus::new();
    let tg = Arc::new(MockTransport::new("tg"));
    let mx = Arc::new(MockTransport::new("mx"));
    bus.register_transport(tg.clone());
    bus.register_transport(mx.clone());

    let mut env = test_envelope(DeliveryMode::Unicast, "tg");
    bus.submit(&mut env).await;

    assert_eq!(tg.deliver_calls.lock().unwrap().len(), 1);
    assert_eq!(mx.deliver_calls.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn test_deliver_fallback_when_transport_missing() {
    let bus = MessageBus::new();
    let tg = Arc::new(MockTransport::new("tg"));
    bus.register_transport(tg.clone());

    let mut env = test_envelope(DeliveryMode::Unicast, "mx");
    env.result_text = "heartbeat alert".to_string();

    bus.submit(&mut env).await;

    assert_eq!(tg.deliver_calls.lock().unwrap().len(), 0);
    assert_eq!(tg.deliver_broadcast_calls.lock().unwrap().len(), 1);
    
    let fallback_env = &tg.deliver_broadcast_calls.lock().unwrap()[0];
    assert!(fallback_env.result_text.contains("mx"));
    assert!(fallback_env.result_text.contains("not available"));
    assert!(fallback_env.result_text.contains("heartbeat alert"));
}

#[tokio::test]
async fn test_busy_main_injection_not_dropped() {
    let pool = Arc::new(LockPool::new(10));
    let bus = Arc::new(MessageBus::with_lock_pool(pool.clone()));

    let chat_lock = pool.get((8452932024, None));
    let lock_guard = chat_lock.lock().await;

    let bus_clone = bus.clone();
    let completed = Arc::new(Mutex::new(false));
    let completed_clone = completed.clone();

    let handle = tokio::spawn(async move {
        let mut env = Envelope::new(Origin::Interagent, 8452932024);
        env.lock_mode = LockMode::Required;
        bus_clone.submit(&mut env).await;
        let mut guard = completed_clone.lock().unwrap();
        *guard = true;
    });

    sleep(Duration::from_millis(50)).await;
    assert!(!*completed.lock().unwrap());

    drop(lock_guard);

    let _ = handle.await;
    assert!(*completed.lock().unwrap());
}
