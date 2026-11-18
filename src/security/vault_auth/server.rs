//! # Loopback HITL Remote Browser HTTP & WebSocket Server (`server.rs`)
//!
//! ## Overview
//! Hosts the token-gated `vault_auth_viewer.html` UI and single-viewer binary JPEG WebSocket
//! stream on `127.0.0.1:0`, coordinating live CDP input, `save_and_exit` persistence, and
//! graceful auto-flush on TTL expiration or `SIGINT`/`SIGTERM`.
//!
//! ## Search Tags
//! #hitl-ws-server, #single-viewer-lock, #jpeg-screencast, #graceful-flush

pub use super::await_session_completion;

use super::cdp::CdpController;
use super::cdp_state::{export_storage_state, ExportedStateSummary};
use super::proxy::{constant_time_eq, is_strict_hex_token};
use super::server_http::{handle_auto_finish, handle_oauth_code, handle_viewer_html, handle_viewer_ping};
use crate::security::vault::current_unix_secs;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path as AxumPath, State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify};

type WsSender = Arc<Mutex<SplitSink<WebSocket, Message>>>;

/// Termination outcome of a HITL remote browser session.
pub enum SessionOutcome {
    /// User explicitly clicked the Save & Finish button (`save_and_exit`).
    SavedByUser(ExportedStateSummary),
    /// Session ended due to TTL expiration or OS termination signal after auto-flushing state.
    Aborted {
        /// Reason string (`"timeout"` or `"signal"`).
        reason: String,
        /// Best-effort state snapshot flushed before closing Chrome.
        flushed: Option<ExportedStateSummary>,
    },
}

/// Shared state for the loopback Axum server.
pub struct AuthServerState {
    /// Expected 256-bit hex token (`64` chars).
    pub token: String,
    /// Display site key label.
    pub site_key: String,
    /// Expiration Unix timestamp in seconds.
    pub expires_at_secs: u64,
    /// CDP port of the local Chrome instance.
    pub cdp_port: u16,
    /// Browser-level CDP WebSocket URL.
    pub browser_ws_url: String,
    /// Target Playwright `storage_state.json` output path.
    pub state_path: PathBuf,
    /// Live CDP input/screenshot controller.
    pub cdp: Arc<CdpController>,
    viewer_gen: AtomicU64,
    viewer_evict: Notify,
    force_frame: Notify,
    saved_done: AtomicBool,
    pub(crate) done_summary: Mutex<Option<ExportedStateSummary>>,
    pub(crate) done_notify: Notify,
}

impl AuthServerState {
    /// Constructs a new loopback server state.
    pub fn new(
        token: String,
        site_key: String,
        expires_at_secs: u64,
        cdp_port: u16,
        browser_ws_url: String,
        state_path: PathBuf,
        cdp: Arc<CdpController>,
    ) -> Arc<Self> {
        Arc::new(Self {
            token,
            site_key,
            expires_at_secs,
            cdp_port,
            browser_ws_url,
            state_path,
            cdp,
            viewer_gen: AtomicU64::new(0),
            viewer_evict: Notify::new(),
            force_frame: Notify::new(),
            saved_done: AtomicBool::new(false),
            done_summary: Mutex::new(None),
            done_notify: Notify::new(),
        })
    }

    pub(crate) fn is_valid_token(&self, candidate: &str) -> bool {
        is_strict_hex_token(candidate)
            && constant_time_eq(candidate.as_bytes(), self.token.as_bytes())
            && current_unix_secs() < self.expires_at_secs
    }

    pub(crate) fn is_saved_done(&self) -> bool {
        self.saved_done.load(Ordering::SeqCst)
    }

    pub(crate) fn mark_auto_finished(&self) {
        self.saved_done.store(true, Ordering::SeqCst);
        self.viewer_evict.notify_waiters();
        self.done_notify.notify_one();
    }
}

/// Binds the loopback HITL auth server on `127.0.0.1:0` and returns `(local_port, server_task)`.
pub async fn spawn_local_auth_server(
    state: Arc<AuthServerState>,
) -> Result<(u16, tokio::task::JoinHandle<()>), String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Failed to bind local auth server: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Failed to inspect local auth server port: {e}"))?
        .port();
    let app = Router::new()
        .route("/auth/:token", get(handle_viewer_html))
        .route("/auth/:token/ping", get(handle_viewer_ping))
        .route("/auth/:token/oauth_code", get(handle_oauth_code))
        .route("/auth/:token/finish", post(handle_auto_finish))
        .route("/auth/:token/ws", get(handle_viewer_ws))
        .with_state(state);
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((port, handle))
}

async fn handle_viewer_ws(
    AxumPath(token): AxumPath<String>,
    State(state): State<Arc<AuthServerState>>,
    ws: WebSocketUpgrade,
) -> Response {
    if !state.is_valid_token(&token) || state.saved_done.load(Ordering::SeqCst) {
        return (StatusCode::FORBIDDEN, "Invalid or expired auth token").into_response();
    }
    let my_gen = state.viewer_gen.fetch_add(1, Ordering::SeqCst) + 1;
    state.viewer_evict.notify_waiters();
    ws.max_message_size(8 * 1024 * 1024)
        .on_upgrade(move |socket| run_viewer_socket(socket, state, my_gen))
}

async fn stream_viewer_frames(state: Arc<AuthServerState>, tx_lock: WsSender, my_gen: u64) {
    let mut last_url = String::new();
    let mut last_tabs = 0usize;
    loop {
        if state.viewer_gen.load(Ordering::SeqCst) != my_gen
            || state.saved_done.load(Ordering::SeqCst)
        {
            break;
        }
        if let Ok(snap) = state.cdp.capture_frame().await {
            let mut tx = tx_lock.lock().await;
            if tx.send(Message::Binary(snap.jpeg_bytes)).await.is_err() {
                break;
            }
            if snap.url != last_url || snap.tab_count != last_tabs {
                last_url = snap.url.clone();
                last_tabs = snap.tab_count;
                let meta = json!({ "url": snap.url, "tab_count": snap.tab_count });
                if tx.send(Message::Text(meta.to_string())).await.is_err() {
                    break;
                }
            }
        }
        tokio::select! {
            _ = state.force_frame.notified() => {
                tokio::time::sleep(Duration::from_millis(35)).await;
            }
            _ = state.viewer_evict.notified() => {
                if state.viewer_gen.load(Ordering::SeqCst) != my_gen {
                    break;
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(190)) => {}
        }
    }
}

async fn process_viewer_inputs(
    mut ws_rx: SplitStream<WebSocket>,
    state: Arc<AuthServerState>,
    tx_lock: WsSender,
    my_gen: u64,
) {
    while let Some(Ok(msg)) = ws_rx.next().await {
        if state.viewer_gen.load(Ordering::SeqCst) != my_gen {
            break;
        }
        let Message::Text(raw) = msg else {
            continue;
        };
        let Ok(val) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if val.get("type").and_then(Value::as_str) == Some("save_and_exit") {
            handle_explicit_save(&state, &tx_lock).await;
            break;
        }
        if val.get("type").and_then(Value::as_str) == Some("copy_out") {
            let reply = match state.cdp.read_copyable_text().await {
                Ok(Some(t)) => json!({ "clipboard": t, "msg": "📋 클립보드 복사 완료! 로컬에 바로 붙여넣을 수 있습니다." }),
                Ok(None) => json!({ "msg": "복사할 선택 영역이나 인증 코드 박스가 없습니다." }),
                Err(e) => json!({ "msg": format!("복사 실패: {e}") }),
            };
            let _ = tx_lock.lock().await.send(Message::Text(reply.to_string())).await;
            continue;
        }
        let note = match state.cdp.dispatch_action(&val).await {
            Ok(n) => n,
            Err(e) => Some(format!("입력 알림: {e}")),
        };
        if let Some(msg_txt) = note {
            let mut tx = tx_lock.lock().await;
            let _ = tx
                .send(Message::Text(json!({ "msg": msg_txt }).to_string()))
                .await;
        }
        state.force_frame.notify_one();
    }
}

async fn run_viewer_socket(socket: WebSocket, state: Arc<AuthServerState>, my_gen: u64) {
    let (ws_tx, ws_rx) = socket.split();
    let tx_lock = Arc::new(Mutex::new(ws_tx));
    let frame_handle = tokio::spawn(stream_viewer_frames(state.clone(), tx_lock.clone(), my_gen));
    process_viewer_inputs(ws_rx, state, tx_lock, my_gen).await;
    frame_handle.abort();
}

async fn handle_explicit_save(state: &Arc<AuthServerState>, tx_lock: &WsSender) {
    match export_storage_state(state.cdp_port, &state.browser_ws_url, &state.state_path).await {
        Ok(summary) => {
            let cookie_count = summary.cookie_count;
            *state.done_summary.lock().await = Some(summary);
            state.saved_done.store(true, Ordering::SeqCst);
            let notice = json!({
                "done": true,
                "cookie_count": cookie_count,
                "msg": format!("✅ 세션 저장 완료 (쿠키 {}개)! 이제 이 창을 닫으셔도 됩니다.", cookie_count),
            });
            {
                let mut tx = tx_lock.lock().await;
                let _ = tx.send(Message::Text(notice.to_string())).await;
                let _ = tx.flush().await;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
            state.done_notify.notify_one();
        }
        Err(e) => {
            let mut tx = tx_lock.lock().await;
            let _ = tx
                .send(Message::Text(
                    json!({ "msg": format!("저장 실패: {e}") }).to_string(),
                ))
                .await;
        }
    }
}
