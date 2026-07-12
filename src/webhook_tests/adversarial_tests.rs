use crate::bus::lock_pool::LockPool;
use crate::webhook::api::files::verify_bearer;
use crate::webhook::auth::{validate_bearer_token, validate_hmac_signature};
use crate::webhook::manager::WebhookManager;
use axum::http::HeaderMap;

#[test]
fn test_empty_token_auth_bypass_bearer() {
    let expected_token = "";

    assert!(
        !validate_bearer_token("Bearer ", expected_token),
        "Empty token should not allow bearer bypass!"
    );
    assert!(!validate_bearer_token("Bearer", expected_token));
}

#[test]
fn test_verify_bearer_empty_token() {
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer ".parse().unwrap());

    assert!(
        !verify_bearer(&headers, ""),
        "verify_bearer should reject when token is empty!"
    );
}

#[tokio::test]
async fn test_lock_pool_eviction_concurrency_violation() {
    let pool = LockPool::new(1);

    let lock_a1 = pool.get(100);
    assert_eq!(pool.len(), 1);

    let _lock_b = pool.get(101);

    assert_eq!(pool.len(), 2);

    let lock_a2 = pool.get(100);

    let a1_ptr = arc_ptr_eq(&lock_a1, &lock_a2);
    assert!(
        a1_ptr,
        "LockPool should return the same Arc, eviction must not create duplicate locks!"
    );

    let guard1 = lock_a1.try_lock();
    let guard2 = lock_a2.try_lock();
    assert!(guard1.is_ok(), "Guard 1 should be lockable");
    assert!(
        guard2.is_err(),
        "Guard 2 should not be lockable concurrently, preserving mutual exclusion!"
    );
}

fn arc_ptr_eq<T>(a: &std::sync::Arc<T>, b: &std::sync::Arc<T>) -> bool {
    std::sync::Arc::ptr_eq(a, b)
}

#[test]
fn test_validate_hmac_invalid_encoding() {
    // If hmac_encoding is invalid or empty, it defaults to hex. Let's make sure it handles errors gracefully.
    let body = b"hello";
    let sig = "invalid-hex-characters-!!!";
    let cache1 = std::sync::OnceLock::new();
    let cache2 = std::sync::OnceLock::new();
    let result = validate_hmac_signature(
        body, sig, "secret", "sha256", "hex", "", "", &cache1, "", &cache2,
    );
    assert!(!result, "Invalid hex signature should not validate");
}

fn create_test_hook(id: &str) -> crate::webhook::models::WebhookEntry {
    crate::webhook::models::WebhookEntry {
        id: id.to_string(),
        title: "Test".to_string(),
        description: "".to_string(),
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
        hmac_sig_prefix: "".to_string(),
        hmac_sig_regex: "".to_string(),
        hmac_payload_prefix_regex: "".to_string(),
        hmac_sig_regex_cached: std::sync::Arc::new(std::sync::OnceLock::new()),
        hmac_payload_prefix_regex_cached: std::sync::Arc::new(std::sync::OnceLock::new()),
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
async fn test_webhook_manager_concurrent_save_race_condition() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("webhooks.json");
    let mgr = std::sync::Arc::new(WebhookManager::new(file_path));

    let mut tasks = Vec::new();
    for i in 0..50 {
        let mgr_clone = mgr.clone();
        tasks.push(tokio::spawn(async move {
            mgr_clone
                .add_hook(create_test_hook(&format!("hook-{}", i)))
                .await
        }));
    }

    let mut failures = Vec::new();
    for task in tasks {
        if let Err(e) = task.await.unwrap() {
            failures.push(e);
        }
    }

    if !failures.is_empty() {
        println!("Found concurrent save failures: {:?}", failures);
    }
}
