//! # Webhook HTTP Server
//!
//! Axum-based HTTP server for webhooks.

use crate::webhook::auth::RateLimiter;
use crate::webhook::manager::WebhookManager;
use axum::{
    extract::{DefaultBodyLimit, Path, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

pub type DispatchHandler = Arc<dyn Fn(String, serde_json::Value) + Send + Sync + 'static>;

pub struct ServerState {
    pub manager: Arc<WebhookManager>,
    pub rate_limiter: RateLimiter,
    pub global_token: String,
    pub dispatch: Option<DispatchHandler>,
    pub max_body_bytes: usize,
}

pub struct WebhookServer {
    state: Arc<ServerState>,
    shutdown_tx: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl WebhookServer {
    pub fn new(
        manager: Arc<WebhookManager>,
        rate_limit_per_minute: usize,
        global_token: String,
        dispatch: Option<DispatchHandler>,
        max_body_bytes: usize,
    ) -> Self {
        Self {
            state: Arc::new(ServerState {
                manager,
                rate_limiter: RateLimiter::new(rate_limit_per_minute),
                global_token,
                dispatch,
                max_body_bytes,
            }),
            shutdown_tx: std::sync::Mutex::new(None),
        }
    }

    pub async fn start(&self, host: &str, port: u16) -> Result<(), String> {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        *self.shutdown_tx.lock().unwrap() = Some(tx);

        let app = Router::new()
            .route("/health", get(handle_health))
            .route("/hooks/:hook_id", post(handle_webhook))
            .layer(DefaultBodyLimit::max(self.state.max_body_bytes))
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
                eprintln!("Webhook Axum server error: {}", e);
            }
        });

        Ok(())
    }

    pub fn stop(&self) {
        if let Some(tx) = self.shutdown_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }
}

async fn handle_health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

async fn handle_webhook(
    Path(hook_id): Path<String>,
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    if !state.rate_limiter.check().await {
        return (StatusCode::TOO_MANY_REQUESTS, Json(serde_json::json!({ "error": "rate_limited" }))).into_response();
    }
    let ct = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
    if ct != "application/json" {
        return (StatusCode::UNSUPPORTED_MEDIA_TYPE, Json(serde_json::json!({ "error": "content_type_must_be_json" }))).into_response();
    }
    let payload: serde_json::Value = match serde_json::from_slice::<serde_json::Value>(&body) {
        Ok(v) if v.is_object() => v,
        Ok(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "body_must_be_object" }))).into_response(),
        _ => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid_json" }))).into_response(),
    };
    let Some(hook) = state.manager.get_hook(&hook_id).await else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "hook_not_found" }))).into_response();
    };
    if !hook.enabled {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "hook_disabled" }))).into_response();
    }
    if !check_auth(&headers, &hook, &body, &state.global_token) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "unauthorized" }))).into_response();
    }
    if let Some(ref dispatch) = state.dispatch {
        let dispatch_clone = dispatch.clone();
        let hook_id_clone = hook_id.clone();
        let payload_clone = payload.clone();
        tokio::spawn(async move {
            dispatch_clone(hook_id_clone, payload_clone);
        });
    }
    (StatusCode::ACCEPTED, Json(serde_json::json!({ "accepted": true, "hook_id": hook_id }))).into_response()
}

fn check_auth(headers: &HeaderMap, hook: &crate::webhook::models::WebhookEntry, body: &[u8], global_token: &str) -> bool {
    let auth_header = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or("");
    let sig_val = if !hook.hmac_header.is_empty() {
        headers.get(&hook.hmac_header).and_then(|v| v.to_str().ok()).unwrap_or("")
    } else {
        ""
    };
    crate::webhook::auth::validate_hook_auth(hook, auth_header, sig_val, body, global_token)
}
