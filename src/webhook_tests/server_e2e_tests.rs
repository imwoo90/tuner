//! # Direct API Server E2E Tests
//!
//! Verification of direct API E2E handshake.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::config::ApiConfig;
use crate::session::key::SessionKey;
use crate::webhook::api::crypto::E2ESession;
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
    let mut len = (header[1] & 0x7F) as usize;
    if len == 126 {
        let mut ext = [0u8; 2];
        stream.read_exact(&mut ext).await.unwrap();
        len = u16::from_be_bytes(ext) as usize;
    } else if len == 127 {
        let mut ext = [0u8; 8];
        stream.read_exact(&mut ext).await.unwrap();
        len = u64::from_be_bytes(ext) as usize;
    }

    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await.unwrap();
    String::from_utf8(payload).unwrap()
}

async fn setup_test_server_and_stream(token: &str) -> (ApiServer, TcpStream) {
    let config = ApiConfig {
        enabled: true,
        host: "127.0.0.1".to_string(),
        port: 0,
        token: token.to_string(),
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
    ws_handshake(&mut stream, port).await;
    (server, stream)
}

#[tokio::test]
async fn test_genuine_server_e2e() {
    let token = "my-secure-token";
    let (server, mut stream) = setup_test_server_and_stream(token).await;

    // 1. Prepare E2E Session on client side
    let mut client_e2e = E2ESession::new();

    // 2. Perform Handshake: send auth
    let auth_msg = serde_json::json!({
        "type": "auth",
        "token": token,
        "e2e_pk": client_e2e.local_pk_b64,
        "chat_id": 123
    });
    send_ws_text(&mut stream, &auth_msg.to_string()).await;

    // 3. Receive auth_ok
    let resp_text = read_ws_text(&mut stream).await;
    let resp_json: Value = serde_json::from_str(&resp_text).unwrap();
    assert_eq!(resp_json.get("type").unwrap().as_str().unwrap(), "auth_ok");

    let server_pk = resp_json.get("e2e_pk").unwrap().as_str().unwrap();
    client_e2e.set_remote_key(server_pk).unwrap();

    // 4. Send encrypted message
    let payload = serde_json::json!({
        "type": "message",
        "text": "hello"
    });
    let enc_payload = client_e2e.encrypt(&payload).unwrap();
    send_ws_text(&mut stream, &enc_payload).await;

    // 5. Read responses: should get delta_1, delta_2, and the final result
    let mut deltas = Vec::new();
    let mut result_text = String::new();

    // Read 3 frames (2 deltas + 1 result)
    for _ in 0..3 {
        let enc_resp = read_ws_text(&mut stream).await;
        let decrypted = client_e2e.decrypt(&enc_resp).unwrap();
        let rtype = decrypted.get("type").unwrap().as_str().unwrap();
        if rtype == "text_delta" {
            deltas.push(decrypted.get("data").unwrap().as_str().unwrap().to_string());
        } else if rtype == "result" {
            result_text = decrypted.get("text").unwrap().as_str().unwrap().to_string();
        }
    }

    assert_eq!(deltas, vec!["delta_1", "delta_2"]);
    assert_eq!(result_text, "echo: hello");

    server.stop().await;
}
