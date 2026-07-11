use crate::bus::envelope::{Envelope, Origin, LockMode, DeliveryMode};
use crate::bus::adapters::{
    from_background_result, from_cron_result, from_heartbeat,
    from_webhook_cron_result, from_webhook_wake, WebhookResult,
};
use crate::background::models::{BackgroundResult, BackgroundResultStatus};

fn fake_background_result(status: BackgroundResultStatus) -> BackgroundResult {
    BackgroundResult {
        task_id: "bg1".to_string(),
        chat_id: 100,
        message_id: 42,
        thread_id: None,
        prompt_preview: "do something".to_string(),
        result_text: "done".to_string(),
        status,
        elapsed_seconds: 1.5,
    }
}

fn fake_webhook_result() -> WebhookResult {
    WebhookResult {
        hook_id: "wh1".to_string(),
        hook_title: "Deploy".to_string(),
        result_text: "deployed".to_string(),
        status: "success".to_string(),
    }
}

#[test]
fn test_from_background_result() {
    let env = from_background_result(&fake_background_result(BackgroundResultStatus::Success));
    assert_eq!(env.origin, Origin::Background);
    assert_eq!(env.chat_id, 100);
    assert_eq!(env.delivery, DeliveryMode::Unicast);
    assert_eq!(env.lock_mode, LockMode::None);
    assert!(!env.needs_injection);
    assert_eq!(env.reply_to_message_id, Some(42));
    assert!(!env.is_error);
}

#[test]
fn test_from_background_result_error() {
    let env = from_background_result(&fake_background_result(BackgroundResultStatus::ErrorTimeout));
    assert!(env.is_error);
}

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

#[test]
fn test_from_webhook_cron_result() {
    let env = from_webhook_cron_result(&fake_webhook_result());
    assert_eq!(env.origin, Origin::WebhookCron);
    assert_eq!(env.delivery, DeliveryMode::Broadcast);
    assert_eq!(env.lock_mode, LockMode::None);
    assert_eq!(env.metadata.get("hook_title").map(|s| s.as_str()), Some("Deploy"));
}

#[test]
fn test_from_webhook_wake() {
    let env = from_webhook_wake(300, "wake up");
    assert_eq!(env.origin, Origin::WebhookWake);
    assert_eq!(env.chat_id, 300);
    assert_eq!(env.prompt, "wake up");
    assert_eq!(env.delivery, DeliveryMode::Unicast);
    assert_eq!(env.lock_mode, LockMode::Required);
}
