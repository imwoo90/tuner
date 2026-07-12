use crate::bus::lock_pool::LockPool;
use crate::webhook::api::server::ApiServer;
use crate::webhook::api::websocket::{ApiAbortHandler, ApiMessageHandler, ApiResult};
use crate::config::ApiConfig;
use crate::session::key::SessionKey;
use crate::webhook::api::crypto::E2ESession;
use serde_json::Value;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_lock_pool_no_premature_eviction() {
    let pool = std::sync::Arc::new(LockPool::new(2)); // Cap at 2
    let counter = std::sync::Arc::new(tokio::sync::Mutex::new(0));
    let mut handles = Vec::new();

    // Spawn 20 tasks competing for key 42
    for _ in 0..20 {
        let pool_clone = pool.clone();
        let counter_clone = counter.clone();
        handles.push(tokio::spawn(async move {
            let lock = pool_clone.get(42);
            let _guard = lock.lock().await;
            let mut val = counter_clone.lock().await;
            *val += 1;
            tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;
        }));
    }

    // Spawn another 20 tasks competing for key 99 to force eviction check
    for _ in 0..20 {
        let pool_clone = pool.clone();
        handles.push(tokio::spawn(async move {
            let lock = pool_clone.get(99);
            let _guard = lock.lock().await;
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    assert_eq!(*counter.lock().await, 20);
}

struct SlowMessageHandler {
    delay: std::time::Duration,
    start_tx: tokio::sync::mpsc::UnboundedSender<()>,
}

#[async_trait::async_trait]
impl ApiMessageHandler for SlowMessageHandler {
    async fn handle_message(
        &self,
        _key: SessionKey,
        text: String,
        on_text_delta: std::sync::Arc<dyn Fn(String) + Send + Sync>,
        _on_tool_activity: std::sync::Arc<dyn Fn(String) + Send + Sync>,
        _on_system_status: std::sync::Arc<dyn Fn(Option<String>) + Send + Sync>,
    ) -> Result<ApiResult, String> {
        let _ = self.start_tx.send(());
        tokio::time::sleep(self.delay).await;
        on_text_delta(format!("done_{}", text));
        Ok(ApiResult {
            text: format!("finished: {}", text),
            stream_fallback: false,
        })
    }
}

struct DummyAbortHandler;

#[async_trait::async_trait]
impl ApiAbortHandler for DummyAbortHandler {
    async fn handle_abort(&self, _chat_id: i64) -> usize {
        1
    }
}

fn get_free_port_adv() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

async fn ws_handshake_adv(stream: &mut tokio::net::TcpStream, port: u16) {
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

async fn send_ws_text_adv(stream: &mut tokio::net::TcpStream, text: &str) {
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

async fn read_ws_text_adv(stream: &mut tokio::net::TcpStream) -> String {
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

fn setup_slow_server(
    token: &str,
    start_tx: tokio::sync::mpsc::UnboundedSender<()>,
) -> (ApiServer, u16) {
    let config = ApiConfig {
        enabled: true,
        host: "127.0.0.1".to_string(),
        port: 0,
        token: token.to_string(),
        chat_id: 123,
        allow_public: false,
    };
    let server = ApiServer::new(config, 123);
    let slow_handler = std::sync::Arc::new(SlowMessageHandler {
        delay: std::time::Duration::from_millis(500),
        start_tx,
    });
    server.set_message_handler(slow_handler);
    server.set_abort_handler(std::sync::Arc::new(DummyAbortHandler));
    let port = get_free_port_adv();
    (server, port)
}

async fn perform_client_handshake(
    stream: &mut tokio::net::TcpStream,
    port: u16,
    token: &str,
    client_e2e: &mut E2ESession,
) {
    ws_handshake_adv(stream, port).await;
    let auth_msg = serde_json::json!({
        "type": "auth",
        "token": token,
        "e2e_pk": client_e2e.local_pk_b64,
        "chat_id": 123
    });
    send_ws_text_adv(stream, &auth_msg.to_string()).await;
    let resp_text = read_ws_text_adv(stream).await;
    let resp_json: serde_json::Value = serde_json::from_str(&resp_text).unwrap();
    assert_eq!(resp_json.get("type").unwrap().as_str().unwrap(), "auth_ok");
    client_e2e.set_remote_key(resp_json.get("e2e_pk").unwrap().as_str().unwrap()).unwrap();
}

async fn send_encrypted_payload(
    stream: &mut tokio::net::TcpStream,
    client_e2e: &E2ESession,
    text: &str,
) {
    let payload = serde_json::json!({
        "type": "message",
        "text": text
    });
    let enc_payload = client_e2e.encrypt(&payload).unwrap();
    send_ws_text_adv(stream, &enc_payload).await;
}

async fn wait_for_second_msg(
    stream: &mut tokio::net::TcpStream,
    client_e2e: &E2ESession,
) -> (bool, bool) {
    let mut got_second_result = false;
    let mut got_first_result = false;
    let start_time = std::time::Instant::now();
    while start_time.elapsed() < std::time::Duration::from_secs(3) {
        let enc_resp = read_ws_text_adv(stream).await;
        if let Ok(decrypted) = client_e2e.decrypt(&enc_resp) {
            let rtype = decrypted.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if rtype == "text_delta" {
                let delta = decrypted.get("data").and_then(|d| d.as_str()).unwrap_or("");
                if delta.contains("first") { got_first_result = true; }
                if delta.contains("second") { got_second_result = true; }
            } else if rtype == "result" {
                let text = decrypted.get("text").and_then(|t| t.as_str()).unwrap_or("");
                if text.contains("first") { got_first_result = true; }
                if text.contains("second") {
                    got_second_result = true;
                    break;
                }
            }
        }
    }
    (got_first_result, got_second_result)
}

#[tokio::test]
async fn test_websocket_loop_preemption() {
    let token = "my-secure-token-adv-1";
    let (start_tx, mut start_rx) = tokio::sync::mpsc::unbounded_channel();
    let (server, port) = setup_slow_server(token, start_tx);
    server.start("127.0.0.1", port).await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut client_e2e = E2ESession::new();
    perform_client_handshake(&mut stream, port, token, &mut client_e2e).await;

    send_encrypted_payload(&mut stream, &client_e2e, "first").await;
    start_rx.recv().await.unwrap();

    send_encrypted_payload(&mut stream, &client_e2e, "second").await;
    let (got_first, got_second) = wait_for_second_msg(&mut stream, &client_e2e).await;

    assert!(!got_first, "First message should have been cancelled/aborted!");
    assert!(got_second, "Second message should have completed successfully!");
    server.stop().await;
}
