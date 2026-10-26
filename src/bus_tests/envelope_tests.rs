#![allow(clippy::unwrap_used, clippy::expect_used)]
use crate::bus::envelope::{Envelope, Origin, LockMode, DeliveryMode};

#[test]
fn test_origin_values() {
    assert_eq!(Origin::Cron.as_str(), "cron");
    assert_eq!(Origin::WebhookWake.as_str(), "webhook_wake");
    assert_eq!(Origin::WebhookCron.as_str(), "webhook_cron");
    assert_eq!(Origin::Heartbeat.as_str(), "heartbeat");
}

#[test]
fn test_envelope_defaults() {
    let env = Envelope::new(Origin::Cron, 100);
    assert_eq!(env.origin, Origin::Cron);
    assert_eq!(env.chat_id, 100);
    assert!(env.topic_id.is_none());
    assert_eq!(env.prompt, "");
    assert_eq!(env.result_text, "");
    assert_eq!(env.status, "");
    assert!(!env.is_error);
    assert_eq!(env.delivery, DeliveryMode::Unicast);
    assert_eq!(env.lock_mode, LockMode::None);
    assert!(!env.needs_injection);
    assert!(env.metadata.is_empty());
    assert_eq!(env.envelope_id, "");
    assert_eq!(env.transport, "tg");
}

#[test]
fn test_envelope_lock_key_without_topic() {
    let env = Envelope::new(Origin::Heartbeat, 42);
    assert_eq!(env.lock_key(), (42, None));
}

#[test]
fn test_envelope_lock_key_with_topic() {
    let mut env = Envelope::new(Origin::WebhookWake, 42);
    env.topic_id = Some(7);
    assert_eq!(env.lock_key(), (42, Some(7)));
}

#[test]
fn test_envelope_metadata_independent() {
    let mut env_a = Envelope::new(Origin::Cron, 1);
    let env_b = Envelope::new(Origin::Cron, 2);
    env_a.metadata.insert("key".to_string(), "value".to_string());
    assert!(!env_b.metadata.contains_key("key"));
}
