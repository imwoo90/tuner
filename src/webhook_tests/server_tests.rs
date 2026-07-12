//! # Webhook REST Server Integration Tests
//!
//! Tests the REST API endpoints using real TCP connections.

use crate::webhook::manager::WebhookManager;
use crate::webhook::models::WebhookEntry;
use crate::webhook::server::WebhookServer;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

fn get_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn make_hook(id: &str) -> WebhookEntry {
    WebhookEntry {
        id: id.to_string(),
        title: "Test Hook".to_string(),
        description: "".to_string(),
        mode: "wake".to_string(),
        prompt_template: "{{msg}}".to_string(),
        enabled: true,
        created_at: "".to_string(),
        task_folder: None,
        auth_mode: "bearer".to_string(),
        token: "hook-token".to_string(),
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
async fn test_server_health_check() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(WebhookManager::new(dir.path().join("webhooks.json")));
    let server = WebhookServer::new(manager, 30, "global-token".to_string(), None, 1024);

    let port = get_free_port();
    server.start("127.0.0.1", port).await.unwrap();

    // Give the server a moment to start
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .await
        .unwrap();

    let mut buf = [0u8; 1024];
    let n = stream.read(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf[..n]);

    assert!(resp.contains("HTTP/1.1 200 OK"));
    assert!(resp.contains("{\"status\":\"ok\"}"));

    server.stop();
}

#[tokio::test]
async fn test_server_webhook_unauthorized() {
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(WebhookManager::new(dir.path().join("webhooks.json")));
    manager.add_hook(make_hook("my-hook")).await.unwrap();

    let server = WebhookServer::new(manager, 30, "global-token".to_string(), None, 1024);
    let port = get_free_port();
    server.start("127.0.0.1", port).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream
        .write_all(
            b"POST /hooks/my-hook HTTP/1.1\r\n\
              Host: 127.0.0.1\r\n\
              Content-Type: application/json\r\n\
              Content-Length: 12\r\n\r\n\
              {\"msg\":\"hi\"}",
        )
        .await
        .unwrap();

    let mut buf = [0u8; 1024];
    let n = stream.read(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf[..n]);

    assert!(resp.contains("HTTP/1.1 401 Unauthorized"));

    server.stop();
}
