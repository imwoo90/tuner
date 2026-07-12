//! # API Server Struct
//!
//! Implements the main ApiServer container and setters.

use crate::webhook::api::files::{handle_file_download, handle_file_upload};
use crate::webhook::api::websocket::{handle_health, handle_websocket};
use axum::{Router, extract::DefaultBodyLimit, routing::get, routing::post};
use std::sync::Arc;

pub struct ApiServerState {
    pub config: crate::config::ApiConfig,
    pub default_chat_id: i64,
    pub lock_pool: Arc<crate::bus::lock_pool::LockPool>,
    pub message_handler: Option<Arc<dyn crate::webhook::api::websocket::ApiMessageHandler>>,
    pub abort_handler: Option<Arc<dyn crate::webhook::api::websocket::ApiAbortHandler>>,
    pub allowed_roots: Option<Vec<std::path::PathBuf>>,
    pub upload_dir: Option<std::path::PathBuf>,
    pub workspace: Option<std::path::PathBuf>,
    pub provider_info: serde_json::Value,
    pub active_state_getter: Option<crate::webhook::api::websocket::ActiveStateGetter>,
    pub next_conn_id: usize,
    pub active_ws: Arc<
        std::sync::Mutex<
            std::collections::HashMap<
                usize,
                tokio::sync::mpsc::UnboundedSender<axum::extract::ws::Message>,
            >,
        >,
    >,
    pub task_hub: Option<Arc<crate::tasks::TaskHub>>,
}

pub struct ApiServer {
    pub state: Arc<std::sync::Mutex<ApiServerState>>,
    shutdown_tx: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl ApiServer {
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
                task_hub: None,
            })),
            shutdown_tx: std::sync::Mutex::new(None),
        }
    }

    pub fn set_message_handler(
        &self,
        handler: Arc<dyn crate::webhook::api::websocket::ApiMessageHandler>,
    ) {
        self.state.lock().unwrap().message_handler = Some(handler);
    }

    pub fn set_abort_handler(
        &self,
        handler: Arc<dyn crate::webhook::api::websocket::ApiAbortHandler>,
    ) {
        self.state.lock().unwrap().abort_handler = Some(handler);
    }

    pub fn set_file_context(
        &self,
        allowed_roots: Vec<std::path::PathBuf>,
        upload_dir: std::path::PathBuf,
        workspace: std::path::PathBuf,
    ) {
        let mut s = self.state.lock().unwrap();
        s.allowed_roots = Some(allowed_roots);
        s.upload_dir = Some(upload_dir);
        s.workspace = Some(workspace);
    }

    pub fn set_provider_info(&self, providers: serde_json::Value) {
        self.state.lock().unwrap().provider_info = providers;
    }

    pub fn set_active_state_getter(
        &self,
        getter: crate::webhook::api::websocket::ActiveStateGetter,
    ) {
        self.state.lock().unwrap().active_state_getter = Some(getter);
    }

    pub fn set_task_hub(&self, hub: Arc<crate::tasks::TaskHub>) {
        self.state.lock().unwrap().task_hub = Some(hub);
    }

    pub async fn start(&self, host: &str, port: u16) -> Result<(), String> {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        *self.shutdown_tx.lock().unwrap() = Some(tx);

        let app = Router::new()
            .route("/health", get(handle_health))
            .route("/ws", get(handle_websocket))
            .route("/files", get(handle_file_download))
            .route("/upload", post(handle_file_upload))
            .route("/tasks/create", post(crate::webhook::api::tasks::handle_task_create))
            .route("/tasks/resume", post(crate::webhook::api::tasks::handle_task_resume))
            .route("/tasks/ask_parent", post(crate::webhook::api::tasks::handle_task_ask_parent))
            .route("/tasks/list", get(crate::webhook::api::tasks::handle_task_list))
            .route("/tasks/cancel", post(crate::webhook::api::tasks::handle_task_cancel))
            .route("/tasks/delete", post(crate::webhook::api::tasks::handle_task_delete))
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

    pub async fn stop(&self) {
        if let Some(tx) = self.shutdown_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
        let active = {
            let s = self.state.lock().unwrap();
            s.active_ws.lock().unwrap().clone()
        };
        for (_, ws_tx) in active {
            let _ = ws_tx.send(axum::extract::ws::Message::Close(None));
        }
    }
}
