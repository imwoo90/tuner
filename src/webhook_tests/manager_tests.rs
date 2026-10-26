//! # Webhook Manager Tests
//!
//! Tests for WebhookManager CRUD operations and atomicity.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::webhook::manager::WebhookManager;
use crate::webhook::models::WebhookEntry;
use serde_json::json;

fn make_temp_manager() -> (tempfile::TempDir, WebhookManager) {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("webhooks.json");
    let mgr = WebhookManager::new(file_path);
    (dir, mgr)
}

fn make_hook(id: &str) -> WebhookEntry {
    WebhookEntry {
        id: id.to_string(),
        title: "Test Hook".to_string(),
        description: "A test webhook".to_string(),
        mode: "wake".to_string(),
        prompt_template: "{{msg}}".to_string(),
        enabled: true,
        created_at: "".to_string(),
        task_folder: None,
        auth_mode: "bearer".to_string(),
        token: "".to_string(),
        hmac_secret: "".to_string(),
        hmac_header: "".to_string(),
        hmac_algorithm: "sha256".to_string(),
        hmac_encoding: "hex".to_string(),
        hmac_sig_prefix: "sha256=".to_string(),
        hmac_sig_regex: "".to_string(),
        hmac_payload_prefix_regex: "".to_string(),
        hmac_sig_regex_cached: std::sync::Arc::new(std::sync::OnceLock::new()),
        hmac_payload_prefix_regex_cached: std::sync::Arc::new(std::sync::OnceLock::new()),
        provider: None,
        model: None,
        effort: None,
        cli_parameters: vec![],
        quiet_start: None,
        quiet_end: None,
        trigger_count: 0,
        last_triggered_at: None,
        last_error: None,
    }
}

#[tokio::test]
async fn test_load_and_get_hook_from_json() {
    let (dir, mgr) = make_temp_manager();
    let hooks_path = dir.path().join("webhooks.json");
    let payload = json!({ "hooks": [make_hook("email-notify")] });
    std::fs::write(&hooks_path, serde_json::to_vec_pretty(&payload).unwrap()).unwrap();

    mgr.reload().await.unwrap();
    let hook = mgr.get_hook("email-notify").await.unwrap();
    assert_eq!(hook.id, "email-notify");
    assert!(mgr.get_hook("nonexistent").await.is_none());
}

#[tokio::test]
async fn test_record_trigger_updates_and_persists_state() {
    let (dir, mgr) = make_temp_manager();
    let hooks_path = dir.path().join("webhooks.json");
    let payload = json!({ "hooks": [make_hook("hook-1")] });
    std::fs::write(&hooks_path, serde_json::to_vec_pretty(&payload).unwrap()).unwrap();

    mgr.reload().await.unwrap();
    mgr.record_trigger("hook-1", Some("timeout".to_string())).await;

    let mgr2 = WebhookManager::new(hooks_path);
    let hook = mgr2.get_hook("hook-1").await.unwrap();
    assert_eq!(hook.trigger_count, 1);
    assert!(hook.last_triggered_at.is_some());
    assert_eq!(hook.last_error.as_deref(), Some("timeout"));
}
