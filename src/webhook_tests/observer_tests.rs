//! # Webhook Observer Tests
//!
//! Tests the background hook execution wiring, dispatching, and lifecycle.

use crate::webhook::manager::WebhookManager;
use crate::webhook::observer::{WebhookObserver, is_quiet_hours_at};
use crate::webhook::models::WebhookEntry;
use crate::config::CliConfig;
use std::sync::Arc;
use chrono::TimeZone;

#[tokio::test]
async fn test_observer_disabled_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(WebhookManager::new(dir.path().join("webhooks.json")));
    let config = Arc::new(crate::config::CliConfig::default());
    let cli = Arc::new(crate::cli::antigravity::AntigravityCli::new(
        (*config).clone(),
    ));

    let observer = WebhookObserver::new(manager, dir.path().join("webhooks.json"), config, cli);
    assert!(observer.start().await.is_ok());
    observer.stop().await;
}

fn make_test_hook(quiet_start: Option<u32>, quiet_end: Option<u32>) -> WebhookEntry {
    WebhookEntry {
        id: "test-hook".to_string(),
        title: "Test Hook".to_string(),
        description: "".to_string(),
        mode: "cron_task".to_string(),
        prompt_template: "{{msg}}".to_string(),
        enabled: true,
        created_at: "".to_string(),
        task_folder: Some("test_task".to_string()),
        auth_mode: "bearer".to_string(),
        token: "test-token".to_string(),
        quiet_start,
        quiet_end,
        ..Default::default()
    }
}

#[test]
fn test_webhook_ignores_heartbeat_quiet_hours() {
    let mut config = CliConfig::default();
    config.telegram_heartbeat_quiet_start = Some(21);
    config.telegram_heartbeat_quiet_end = Some(8);
    config.user_timezone = Some("UTC".to_string());

    let hook = make_test_hook(None, None);
    let now = chrono::Utc.with_ymd_and_hms(2025, 6, 15, 23, 30, 0).unwrap();
    assert!(!is_quiet_hours_at(&hook, &config, now));
}

#[test]
fn test_webhook_runs_during_active_hours() {
    let mut config = CliConfig::default();
    config.user_timezone = Some("UTC".to_string());

    let hook = make_test_hook(Some(10), Some(16));
    let now = chrono::Utc.with_ymd_and_hms(2025, 6, 15, 17, 0, 0).unwrap();
    assert!(!is_quiet_hours_at(&hook, &config, now));
}

#[test]
fn test_webhook_respects_task_specific_quiet_hours() {
    let mut config = CliConfig::default();
    config.user_timezone = Some("UTC".to_string());

    let hook = make_test_hook(Some(10), Some(16));
    let now = chrono::Utc.with_ymd_and_hms(2025, 6, 15, 14, 0, 0).unwrap();
    assert!(is_quiet_hours_at(&hook, &config, now));
}
