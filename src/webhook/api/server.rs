//! # API Ingress Router Configuration
//!
//! ## Overview
//! Composes Axum application routes, sets rate limits, registers websocket services, and binds to ports.
//!
//! ## Search Tags
//! #server

use crate::webhook::api::files::{handle_file_download, handle_file_upload};
use crate::webhook::api::websocket::{handle_health, handle_websocket};
use axum::{Router, extract::DefaultBodyLimit, routing::get, routing::post};
use std::sync::Arc;

/// Represents `ApiServerState` (api server state).
pub struct ApiServerState {
    /// Config.
    pub config: crate::config::ApiConfig,
    /// Default chat id.
    pub default_chat_id: i64,
    /// Lock pool.
    pub lock_pool: Arc<crate::bus::lock_pool::LockPool>,
    /// Message handler.
    pub message_handler: Option<Arc<dyn crate::webhook::api::websocket::ApiMessageHandler>>,
    /// Abort handler.
    pub abort_handler: Option<Arc<dyn crate::webhook::api::websocket::ApiAbortHandler>>,
    /// Allowed roots.
    pub allowed_roots: Option<Vec<std::path::PathBuf>>,
    /// Upload dir.
    pub upload_dir: Option<std::path::PathBuf>,
    /// Workspace.
    pub workspace: Option<std::path::PathBuf>,
    /// Provider info.
    pub provider_info: serde_json::Value,
    /// Active state getter.
    pub active_state_getter: Option<crate::webhook::api::websocket::ActiveStateGetter>,
    /// Next conn id.
    pub next_conn_id: usize,
    /// Active ws.
    pub active_ws: Arc<
        std::sync::Mutex<
            std::collections::HashMap<
                usize,
                tokio::sync::mpsc::UnboundedSender<axum::extract::ws::Message>,
            >,
        >,
    >,

}

/// Represents `ApiServer` (api server).
pub struct ApiServer {
    /// State.
    pub state: Arc<std::sync::Mutex<ApiServerState>>,
    shutdown_tx: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl ApiServer {
    /// Creates a new instance.
    pub fn new(config: crate::config::ApiConfig, default_chat_id: i64) -> Self {
        Self {
            state: Arc::new(std::sync::Mutex::new(ApiServerState {
                config,
                default_chat_id,
                lock_pool: Arc::new(crate::bus::lock_pool::LockPool::new_default()),
                message_handler: None,
                abort_handler: None,
                allowed_roots: None,
                upload_dir: None,
                workspace: None,
                provider_info: serde_json::json!([]),
                active_state_getter: None,
                next_conn_id: 0,
                active_ws: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),

            })),
            shutdown_tx: std::sync::Mutex::new(None),
        }
    }

    /// Executes `set_message_handler` (set message handler).
    pub fn set_message_handler(
        &self,
        handler: Arc<dyn crate::webhook::api::websocket::ApiMessageHandler>,
    ) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).message_handler = Some(handler);
    }

    /// Executes `set_abort_handler` (set abort handler).
    pub fn set_abort_handler(
        &self,
        handler: Arc<dyn crate::webhook::api::websocket::ApiAbortHandler>,
    ) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).abort_handler = Some(handler);
    }

    /// Executes `set_file_context` (set file context).
    pub fn set_file_context(
        &self,
        allowed_roots: Vec<std::path::PathBuf>,
        upload_dir: std::path::PathBuf,
        workspace: std::path::PathBuf,
    ) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.allowed_roots = Some(allowed_roots);
        s.upload_dir = Some(upload_dir);
        s.workspace = Some(workspace);
    }

    /// Executes `set_provider_info` (set provider info).
    pub fn set_provider_info(&self, providers: serde_json::Value) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).provider_info = providers;
    }

    /// Executes `set_active_state_getter` (set active state getter).
    pub fn set_active_state_getter(
        &self,
        getter: crate::webhook::api::websocket::ActiveStateGetter,
    ) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).active_state_getter = Some(getter);
    }


    /// Executes `start` (start).
    pub async fn start(&self, host: &str, port: u16) -> Result<(), String> {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        *self.shutdown_tx.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);

        let app = Router::new()
            .route("/health", get(handle_health))
            .route("/ws", get(handle_websocket))
            .route("/files", get(handle_file_download))
            .route("/upload", post(handle_file_upload))

            .layer(DefaultBodyLimit::max(50 * 1024 * 1024))
            .with_state(self.state.clone());

        let addr = format!("{}:{}", host, port);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| e.to_string())?;

        let server_future = axum::serve(listener, app).with_graceful_shutdown(async move {
            let _ = rx.await;
        });

        tokio::spawn(async move {
            if let Err(e) = server_future.await {
                eprintln!("API Axum server error: {}", e);
            }
        });

        Ok(())
    }

    /// Executes `stop` (stop).
    pub async fn stop(&self) {
        if let Some(tx) = self.shutdown_tx.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = tx.send(());
        }
        let active = {
            let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.active_ws.lock().unwrap_or_else(|e| e.into_inner()).clone()
        };
        for (_, ws_tx) in active {
            let _ = ws_tx.send(axum::extract::ws::Message::Close(None));
        }
    }
}
