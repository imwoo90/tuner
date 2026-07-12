//! # Webhook Observer Tests
//!
//! Tests the background hook execution wiring, dispatching, and lifecycle.

use crate::webhook::manager::WebhookManager;
use crate::webhook::observer::WebhookObserver;
use std::sync::Arc;

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
