//! # Message Bus Module
//!
//! ## Overview
//! Provides the `MessageBus` implementation for submitting envelopes, acquiring chat locks,
//! executing prompt injection, and delivering messages to registered transports (such as Telegram).
//! Handles cascading transport fallback and error resilience.
//!
//! ## Search Tags
//! #bus

use std::sync::{Arc, Mutex};
use async_trait::async_trait;
use super::envelope::{Envelope, LockMode, DeliveryMode};
use super::lock_pool::LockPool;

#[async_trait]
/// Defines the `Transport` (transport) interface.
pub trait Transport: Send + Sync {
    /// Return the identifier of this transport (e.g. "tg").
    fn transport_name(&self) -> &str;

    /// Deliver a unicast envelope.
    async fn deliver(&self, envelope: &Envelope) -> Result<(), String>;

    /// Deliver a broadcast envelope.
    async fn deliver_broadcast(&self, envelope: &Envelope) -> Result<(), String>;
}

#[async_trait]
/// Defines the `PromptInjector` (prompt injector) interface.
pub trait PromptInjector: Send + Sync {
    /// Inject the given prompt into the active CLI session.
    async fn inject_prompt(
        &self,
        prompt: &str,
        chat_id: i64,
        label: &str,
        topic_id: Option<i64>,
        transport: &str,
    ) -> Result<String, String>;
}

/// Represents `MessageBus` (message bus).
pub struct MessageBus {
    transports: Mutex<Vec<Arc<dyn Transport>>>,
    lock_pool: Arc<LockPool>,
    injector: Mutex<Option<Arc<dyn PromptInjector>>>,
}

#[cfg(test)]
impl Default for MessageBus {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageBus {
    /// Create a new MessageBus with an isolated LockPool for unit testing.
    #[cfg(test)]
    pub fn new() -> Self {
        Self::with_lock_pool(Arc::new(LockPool::new()))
    }

    /// Create a new MessageBus sharing the given LockPool.
    pub fn with_lock_pool(lock_pool: Arc<LockPool>) -> Self {
        Self {
            transports: Mutex::new(Vec::new()),
            lock_pool,
            injector: Mutex::new(None),
        }
    }

    /// Register a transport with the bus.
    pub fn register_transport(&self, transport: Arc<dyn Transport>) {
        self.transports.lock().unwrap_or_else(|e| e.into_inner()).push(transport);
    }

    /// Set the active prompt injector.
    pub fn set_injector(&self, injector: Arc<dyn PromptInjector>) {
        *self.injector.lock().unwrap_or_else(|e| e.into_inner()) = Some(injector);
    }

    /// Submit an envelope to the bus for processing and delivery.
    pub async fn submit(&self, envelope: &mut Envelope) {
        if envelope.envelope_id.is_empty() {
            envelope.envelope_id = generate_envelope_id();
        }

        if envelope.lock_mode == LockMode::Required {
            let key = envelope.lock_key();
            let lock = self.lock_pool.get(key);
            let _guard = lock.lock().await;
            self.process(envelope).await;
        } else {
            self.process(envelope).await;
        }
    }

    async fn process(&self, envelope: &mut Envelope) {
        let injector = self.injector.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if envelope.needs_injection && !envelope.prompt.is_empty()
            && let Some(ref inj) = injector {
                let label = format!("{}:{}", envelope.origin.as_str(), &envelope.envelope_id);
                match inj.inject_prompt(
                    &envelope.prompt,
                    envelope.chat_id,
                    &label,
                    envelope.topic_id,
                    &envelope.transport,
                ).await {
                    Ok(response) => {
                        envelope.result_text = response;
                    }
                    Err(e) => {
                        envelope.is_error = true;
                        if envelope.result_text.is_empty() {
                            envelope.result_text = format!(
                                "Error processing {} result: {}",
                                envelope.origin.as_str(),
                                e
                            );
                        }
                    }
                }
            }

        self.deliver(envelope).await;
    }

    async fn deliver(&self, envelope: &Envelope) {
        let transports = self.transports.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if transports.is_empty() {
            return;
        }

        if envelope.delivery == DeliveryMode::Broadcast {
            for transport in transports {
                let _ = transport.deliver_broadcast(envelope).await;
            }
            return;
        }

        let target_transport = normalize_transport_name(&envelope.transport);
        let mut matching = Vec::new();
        let mut others = Vec::new();

        for t in transports {
            if normalize_transport_name(t.transport_name()) == target_transport {
                matching.push(t);
            } else {
                others.push(t);
            }
        }

        for transport in &matching {
            let _ = transport.deliver(envelope).await;
        }

        if matching.is_empty() && !others.is_empty() {
            let mut fallback_env = Envelope::new(envelope.origin, envelope.chat_id);
            fallback_env.delivery = DeliveryMode::Broadcast;
            fallback_env.result_text = format!(
                "**Delivery fallback**\n\nTarget transport '{}' is not available.\n\n---\n{}",
                envelope.transport,
                envelope.result_text
            );
            fallback_env.status = envelope.status.clone();
            fallback_env.lock_mode = envelope.lock_mode;
            fallback_env.metadata = envelope.metadata.clone();
            fallback_env.topic_id = envelope.topic_id;

            let _ = others[0].deliver_broadcast(&fallback_env).await;
        }
    }
}

fn normalize_transport_name(name: &str) -> &str {
    if name.eq_ignore_ascii_case("telegram") {
        "tg"
    } else {
        name
    }
}

fn generate_envelope_id() -> String {
    use std::io::Read;
    let seed = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u32;
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let mut buf = [0u8; 6];
        if f.read_exact(&mut buf).is_ok() {
            return buf.iter().map(|b| format!("{:02x}", b)).collect();
        }
    }
    format!("{:012x}", seed)
}
