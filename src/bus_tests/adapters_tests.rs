#![allow(clippy::unwrap_used, clippy::expect_used)]
use crate::bus::envelope::{Origin, LockMode, DeliveryMode};
use crate::bus::adapters::{from_cron_result, from_heartbeat};

#[test]
fn test_from_cron_result() {
    let env = from_cron_result("Daily Report", "all good", "success", None, None, None);
    assert_eq!(env.origin, Origin::Cron);
    assert_eq!(env.chat_id, 0);
    assert_eq!(env.delivery, DeliveryMode::Broadcast);
    assert_eq!(env.lock_mode, LockMode::None);
    assert_eq!(env.metadata.get("title").map(|s| s.as_str()), Some("Daily Report"));
    assert_eq!(env.result_text, "all good");
}

#[test]
fn test_from_cron_result_with_chat_id_creates_unicast() {
    let env = from_cron_result("Title", "Result", "success", Some(12345), Some(99), None);
    assert_eq!(env.chat_id, 12345);
    assert_eq!(env.topic_id, Some(99));
    assert_eq!(env.delivery, DeliveryMode::Unicast);
}

#[test]
fn test_from_cron_result_without_chat_id_broadcasts() {
    let env = from_cron_result("Title", "Result", "success", None, None, None);
    assert_eq!(env.chat_id, 0);
    assert_eq!(env.delivery, DeliveryMode::Broadcast);
}

#[test]
fn test_from_heartbeat() {
    let env = from_heartbeat(200, "alert text", None, None);
    assert_eq!(env.origin, Origin::Heartbeat);
    assert_eq!(env.chat_id, 200);
    assert!(env.topic_id.is_none());
    assert_eq!(env.delivery, DeliveryMode::Unicast);
    assert_eq!(env.lock_mode, LockMode::None);
    assert_eq!(env.result_text, "alert text");
}

#[test]
fn test_from_heartbeat_with_topic_id() {
    let env = from_heartbeat(-1001, "group alert", Some(42), None);
    assert_eq!(env.origin, Origin::Heartbeat);
    assert_eq!(env.chat_id, -1001);
    assert_eq!(env.topic_id, Some(42));
    assert_eq!(env.delivery, DeliveryMode::Unicast);
    assert_eq!(env.result_text, "group alert");
}
