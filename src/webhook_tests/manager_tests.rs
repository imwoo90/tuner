//! # Webhook Manager Tests
//!
//! Tests for WebhookManager CRUD operations and atomicity.

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
        provider: None,
        model: None,
        reasoning_effort: None,
        cli_parameters: vec![],
        quiet_start: None,
        quiet_end: None,
        dependency: None,
        trigger_count: 0,
        last_triggered_at: None,
        last_error: None,
    }
}

#[tokio::test]
async fn test_add_hook_saves_to_json() {
    let (_dir, mgr) = make_temp_manager();
    mgr.add_hook(make_hook("email-notify")).await.unwrap();

    let hooks = mgr.list_hooks().await;
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].id, "email-notify");
}

#[tokio::test]
async fn test_add_duplicate_raises() {
    let (_dir, mgr) = make_temp_manager();
    mgr.add_hook(make_hook("email-notify")).await.unwrap();
    assert!(mgr.add_hook(make_hook("email-notify")).await.is_err());
}

#[tokio::test]
async fn test_remove_hook() {
    let (_dir, mgr) = make_temp_manager();
    mgr.add_hook(make_hook("email-notify")).await.unwrap();
    assert!(mgr.remove_hook("email-notify").await.unwrap());
    assert_eq!(mgr.list_hooks().await.len(), 0);
}

#[tokio::test]
async fn test_remove_nonexistent_returns_false() {
    let (_dir, mgr) = make_temp_manager();
    assert!(!mgr.remove_hook("missing").await.unwrap());
}

#[tokio::test]
async fn test_update_hook() {
    let (_dir, mgr) = make_temp_manager();
    mgr.add_hook(make_hook("hook-1")).await.unwrap();

    let updates = json!({
        "title": "New Title",
        "enabled": false
    });
    assert!(mgr.update_hook("hook-1", &updates).await.unwrap());

    let hook = mgr.get_hook("hook-1").await.unwrap();
    assert_eq!(hook.title, "New Title");
    assert!(!hook.enabled);
}
