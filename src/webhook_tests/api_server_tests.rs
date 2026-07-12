//! # Direct API Server Integration and Security Tests
//!
//! Tests direct API server endpoints, token authentication, directory traversal,
//! E2E encryption handshake, and concurrent request handling.

use crate::config::ApiConfig;
use crate::session::key::SessionKey;
use crate::webhook::api::server::ApiServer;
use crate::webhook::api::websocket::{ApiAbortHandler, ApiMessageHandler, ApiResult};
use serde_json::Value;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

fn get_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

struct MockMessageHandler;

#[async_trait::async_trait]
impl ApiMessageHandler for MockMessageHandler {
    async fn handle_message(
        &self,
        _key: SessionKey,
        text: String,
        on_text_delta: Arc<dyn Fn(String) + Send + Sync>,
        _on_tool_activity: Arc<dyn Fn(String) + Send + Sync>,
        _on_system_status: Arc<dyn Fn(Option<String>) + Send + Sync>,
    ) -> Result<ApiResult, String> {
        on_text_delta("delta_1".to_string());
        on_text_delta("delta_2".to_string());
        Ok(ApiResult {
            text: format!("echo: {}", text),
            stream_fallback: false,
        })
    }
}

struct MockAbortHandler;

#[async_trait::async_trait]
impl ApiAbortHandler for MockAbortHandler {
    async fn handle_abort(&self, _chat_id: i64) -> usize {
        1
    }
}

async fn ws_handshake(stream: &mut TcpStream, port: u16) {
    let req = format!(
        "GET /ws HTTP/1.1\r\n\
         Host: 127.0.0.1:{}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n",
        port
    );
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = [0u8; 1024];
    let n = stream.read(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf[..n]);
    assert!(resp.contains("101 Switching Protocols"));
}

async fn send_ws_text(stream: &mut TcpStream, text: &str) {
    let payload = text.as_bytes();
    let len = payload.len();
    assert!(len < 126, "Only small payloads supported in test helper");

    let mut frame = Vec::new();
    frame.push(0x81); // FIN, text frame
    frame.push(0x80 | (len as u8)); // Mask bit set, length

    let mask: [u8; 4] = [0x11, 0x22, 0x33, 0x44];
    frame.extend_from_slice(&mask);

    for (i, &byte) in payload.iter().enumerate() {
        frame.push(byte ^ mask[i % 4]);
    }

    stream.write_all(&frame).await.unwrap();
}

async fn read_ws_text(stream: &mut TcpStream) -> String {
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).await.unwrap();
    let len = (header[1] & 0x7F) as usize;

    // Check mask bit (should be 0 for server to client)
    assert_eq!(header[1] & 0x80, 0);

    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await.unwrap();
    String::from_utf8(payload).unwrap()
}

#[tokio::test]
async fn test_api_server_health_check() {
    let config = ApiConfig {
        enabled: true,
        host: "127.0.0.1".to_string(),
        port: 0,
        token: "test-token".to_string(),
        chat_id: 123,
        allow_public: false,
    };
    let server = ApiServer::new(config, 123);
    server.set_message_handler(Arc::new(MockMessageHandler));
    server.set_abort_handler(Arc::new(MockAbortHandler));
    let port = get_free_port();
    server.start("127.0.0.1", port).await.unwrap();
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
    assert!(resp.contains("connections"));

    server.stop().await;
}

#[tokio::test]
async fn test_api_server_token_auth_bypass_vulnerability() {
    let config = ApiConfig {
        enabled: true,
        host: "127.0.0.1".to_string(),
        port: 0,
        token: "".to_string(), // Empty token!
        chat_id: 123,
        allow_public: false,
    };
    let server = ApiServer::new(config, 123);
    let port = get_free_port();
    server.start("127.0.0.1", port).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Connect to WS and send auth message with empty token
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    ws_handshake(&mut stream, port).await;

    let client_pk = base64::Engine::encode(&base64::prelude::BASE64_STANDARD, &[0u8; 32]);
    let auth_msg = serde_json::json!({
        "type": "auth",
        "token": "",
        "e2e_pk": client_pk,
        "chat_id": 123
    });
    send_ws_text(&mut stream, &auth_msg.to_string()).await;

    // Read response - it should fail because token is empty
    let resp_text = read_ws_text(&mut stream).await;
    let resp_json: Value = serde_json::from_str(&resp_text).unwrap();
    assert_eq!(
        resp_json.get("type").unwrap().as_str().unwrap(),
        "error",
        "Should reject empty token: {}",
        resp_text
    );
    assert_eq!(
        resp_json.get("code").unwrap().as_str().unwrap(),
        "auth_failed"
    );

    server.stop().await;
}

#[tokio::test]
async fn test_api_server_directory_traversal_vulnerability() {
    let tmp = tempfile::tempdir().unwrap();
    let secret_file = tmp.path().join("secret.txt");
    std::fs::write(&secret_file, "supersecret").unwrap();

    let config = ApiConfig {
        enabled: true,
        host: "127.0.0.1".to_string(),
        port: 0,
        token: "my-token".to_string(),
        chat_id: 123,
        allow_public: false,
    };
    let server = ApiServer::new(config, 123);

    // Notice we do NOT call set_file_context, so allowed_roots is None
    let port = get_free_port();
    server.start("127.0.0.1", port).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Access the secret file outside any allowed root since roots is None
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req = format!(
        "GET /files?path={} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer my-token\r\n\r\n",
        secret_file.to_str().unwrap()
    );
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = [0u8; 1024];
    let n = stream.read(&mut buf).await.unwrap();
    let resp = String::from_utf8_lossy(&buf[..n]);

    // Should return 403 Forbidden
    assert!(
        resp.contains("HTTP/1.1 403 Forbidden"),
        "Should reject traversal check and return 403 Forbidden: {}",
        resp
    );
    assert!(
        !resp.contains("supersecret"),
        "Should not contain secret file contents"
    );

    server.stop().await;
}
