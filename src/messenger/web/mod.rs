//! # Responsive Web UI & PWA Control Plane (`web/mod.rs`)
//!
//! ## Overview
//! Hosts the embedded pure-Rust Web Dashboard (`:8743/dashboard`), unified session & chat
//! REST/WebSocket APIs, PBKDF2 + Magic Link auth, subagent tree, and operational panels.
//!
//! ## Submodules
//! - [`api_ask`] (`api_ask.rs`): Interactive `AskQuestion` card answers and resume.
//! - [`api_ask_helpers`] (`api_ask_helpers.rs`): PTY keystroke and step helpers for AskQuestion.
//! - [`api_chat`] (`api_chat.rs`): Prompt dispatch, soft `/stop`, and `/new` resets.
//! - [`api_files`] (`api_files.rs`): Sandboxed `/file` preview/raw and Zero-Knowledge `/vault`.
//! - [`api_history`] (`api_history.rs`): JSONL chat history, active ask, and multipart upload.
//! - [`api_hitl`] (`api_hitl.rs`): In-dashboard HITL remote browser status and launcher.
//! - [`api_models`] (`api_models.rs`): Live provider model & reasoning effort catalog.
//! - [`api_ops`] (`api_ops.rs`): Structured `/usage` and `/cron` operational APIs.
//! - [`api_sessions`] (`api_sessions.rs`): Session CRUD and provider/model/effort config.
//! - [`assets_http`] (`assets_http.rs`): Embedded PWA HTML, CSS, JS, manifest, and SW handlers.
//! - [`auth`] (`auth.rs`): Persistent `0600` auth operations, Magic Links, and PBKDF2 PINs.
//! - [`auth_http`] (`auth_http.rs`): Auth HTTP endpoints, cookie parsing, and CSRF checks.
//! - [`auth_store`] (`auth_store.rs`): `flock`-guarded `dashboard_auth.json` I/O and `429` lockouts.
//! - [`cli_dash`] (`cli_dash.rs`): `tuner --dashboard` CLI and Telegram `/dashboard` command.
//! - [`events`] (`events.rs`): Real-time `WebEventHub` broadcast channel and stream cache.
//! - [`events_bridge`] (`events_bridge.rs`): Compact Telegram & Cron hooks emitting Web events.
//! - [`subagent_tree`] (`subagent_tree.rs`): Warm PID and recursive `.system_generated/` inspector.
//! - [`tg_topics`] (`tg_topics.rs`): Live Telegram `createForumTopic` / `editForumTopic` / `deleteForumTopic` sync.
//! - [`turn_boot`] (`turn_boot.rs`): Session init, auto-handover, and observer attach.
//! - [`turn_runner`] (`turn_runner.rs`): Lock-pooled Web turn dispatch and Telegram mirror.
//! - [`turn_stream`] (`turn_stream.rs`): Live `StreamEvent` pump and result persistence.
//! - [`ws`] (`ws.rs`): Cookie/CSRF/Origin-verified `/dashboard/api/ws` WebSocket handler.
//!
//! ## Search Tags
//! #web-dashboard, #pwa-ui, #control-plane, #axum-router

pub mod api_ask;
pub mod api_ask_helpers;
pub mod api_chat;
pub mod api_files;
pub mod api_history;
pub mod api_hitl;
pub mod api_models;
pub mod api_ops;
pub mod api_sessions;
pub mod assets_http;
pub mod auth;
pub mod auth_http;
pub mod auth_store;
pub mod cli_dash;
pub mod events;
pub mod events_bridge;
pub mod subagent_tree;
pub mod tg_topics;
pub mod turn_boot;
pub mod turn_runner;
pub mod turn_stream;
pub mod ws;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod dashboard_tests;

use crate::bus::bus::{MessageBus, Transport};
use crate::bus::envelope::Envelope;
use crate::cli::antigravity::AntigravityCli;
use crate::config::CliConfig;
use crate::cron::manager::CronManager;
use crate::messenger::telegram::TopicNameCache;
use crate::session::{data::SessionData, key::SessionKey, manager::SessionManager};
use auth::WebAuthStore;
use axum::{
    extract::DefaultBodyLimit,
    http::StatusCode,
    response::Response,
    routing::{delete, get, patch, post},
    Extension, Router,
};
use events::{WebEvent, WebEventHub};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use teloxide::Bot;
use tokio::sync::RwLock;

/// Shared control-plane state powering all `/dashboard` HTTP and WebSocket endpoints.
pub struct WebAppCtx {
    /// Active profile name (`default`, etc.).
    pub profile: String,
    /// Tuner home directory (`~/.tuner`).
    pub tuner_home: PathBuf,
    /// Per-profile encrypted vault directory.
    pub vault_dir: PathBuf,
    /// Active workspace root directory.
    pub working_dir: PathBuf,
    /// Loaded profile configuration.
    pub config: Arc<CliConfig>,
    /// Shared session manager and `LockPool`.
    pub sessions: Arc<SessionManager>,
    /// Shared multi-provider CLI orchestrator.
    pub cli: Arc<AntigravityCli>,
    /// Shared cron manager.
    pub cron_manager: Arc<CronManager>,
    /// Forum topic name cache.
    pub topic_cache: Arc<TopicNameCache>,
    /// Shared internal message bus.
    pub bus: Arc<MessageBus>,
    /// Optional attached Telegram bot handle (`None` in Web-Standalone mode).
    pub bot: RwLock<Option<Bot>>,
    /// Real-time WebSocket event broadcast hub.
    pub events: Arc<WebEventHub>,
    /// Persistent `0600` Web authentication store.
    pub auth: Arc<WebAuthStore>,
}

impl WebAppCtx {
    /// Attaches a validated Telegram `Bot` handle after startup initialization succeeds.
    pub async fn set_bot(&self, bot: Bot) {
        *self.bot.write().await = Some(bot);
    }

    /// Clones the attached Telegram `Bot` handle if Co-Resident mode is active.
    pub async fn bot_clone(&self) -> Option<Bot> {
        self.bot.read().await.clone()
    }
}

static GLOBAL_WEB_CTX: OnceLock<Arc<WebAppCtx>> = OnceLock::new();

/// Registers the global [`WebAppCtx`] instance for `:8743` and Telegram hooks.
pub fn init_global_web_ctx(ctx: Arc<WebAppCtx>) {
    let _ = GLOBAL_WEB_CTX.set(ctx);
}

/// Returns the global [`WebAppCtx`] instance once initialized.
pub fn global_web_ctx() -> Option<Arc<WebAppCtx>> {
    GLOBAL_WEB_CTX.get().cloned()
}

pub(crate) fn resolve_ctx(ext: Option<Extension<Arc<WebAppCtx>>>) -> Result<Arc<WebAppCtx>, Box<Response>> {
    if let Some(Extension(c)) = ext {
        return Ok(c);
    }
    global_web_ctx().ok_or_else(|| {
        Box::new(auth_http::json_err(StatusCode::SERVICE_UNAVAILABLE, "Dashboard context is still initializing"))
    })
}

/// MessageBus [`Transport`] adapter delivering cron & heartbeat results to the Web Dashboard.
pub struct WebTransport {
    ctx: Arc<WebAppCtx>,
}

impl WebTransport {
    /// Creates a new `WebTransport` bound to `ctx`.
    pub fn new(ctx: Arc<WebAppCtx>) -> Self {
        Self { ctx }
    }

    async fn pick_fallback_web_key(&self, topic_hint: Option<i64>) -> SessionKey {
        if let Some(tid) = topic_hint.filter(|&t| t > 0) {
            return SessionKey::web(tid);
        }
        let all = self.ctx.sessions.list_all().await.unwrap_or_default();
        if let Some(s) = all.iter().find(|s| s.transport == "web") {
            return s.session_key();
        }
        if let Some(s) = all.first() {
            return s.session_key();
        }
        SessionKey::web(1)
    }

    async fn resolve_target_key(&self, envelope: &Envelope, is_broadcast: bool) -> Option<SessionKey> {
        let has_bot = self.ctx.bot.read().await.is_some();
        if envelope.transport == "web" {
            if envelope.chat_id != 0 {
                let tg_key = SessionKey::telegram(envelope.chat_id, envelope.topic_id);
                if self.ctx.sessions.get_active(&tg_key).await.ok().flatten().is_some() {
                    return Some(tg_key);
                }
            }
            return Some(self.pick_fallback_web_key(envelope.topic_id).await);
        }
        if is_broadcast && has_bot {
            return None;
        }
        if envelope.chat_id != 0 {
            let tg_key = SessionKey::telegram(envelope.chat_id, envelope.topic_id);
            if self.ctx.sessions.get_active(&tg_key).await.ok().flatten().is_some() {
                return Some(tg_key);
            }
        }
        Some(self.pick_fallback_web_key(envelope.topic_id).await)
    }

    async fn deliver_resolved(&self, key: SessionKey, envelope: &Envelope) -> Result<(), String> {
        let clean_text = strip_bus_fallback_banner(&envelope.result_text);
        if let Ok(Some(sess)) = self.ctx.sessions.get_active(&key).await {
            let prov = SessionData::effective_provider(&sess.provider, &self.ctx.config.provider);
            let mut sid = sess.get_session_id(prov);
            if sid.is_empty() {
                sid = sess.get_past_session_ids(prov).last().cloned().unwrap_or_default();
            }
            if !sid.is_empty() {
                crate::messenger::telegram::history::log_telegram_message(
                    &self.ctx.working_dir,
                    &sid,
                    (sess.topic_id, sess.topic_name.as_deref()),
                    "bot",
                    None,
                    clean_text,
                    (!envelope.is_error, envelope.is_error.then_some(envelope.status.as_str())),
                );
            }
        }
        let files = api_files::filter_safe_preview_paths(&self.ctx, clean_text);
        self.ctx
            .events
            .publish(WebEvent::TurnResult {
                session_key: key.storage_key(),
                session_id: None,
                text: clean_text.to_string(),
                is_error: envelope.is_error,
                files,
                timestamp: chrono::Utc::now().to_rfc3339(),
            })
            .await;
        Ok(())
    }
}

fn strip_bus_fallback_banner(text: &str) -> &str {
    const PREFIX: &str = "**Delivery fallback**\n\nTarget transport '";
    if text.starts_with(PREFIX)
        && let Some((_, rest)) = text.split_once("\n\n---\n")
    {
        return rest;
    }
    text
}

#[async_trait::async_trait]
impl Transport for WebTransport {
    fn transport_name(&self) -> &str {
        "web"
    }

    async fn deliver(&self, envelope: &Envelope) -> Result<(), String> {
        if let Some(key) = self.resolve_target_key(envelope, false).await {
            self.deliver_resolved(key, envelope).await?;
        }
        Ok(())
    }

    async fn deliver_broadcast(&self, envelope: &Envelope) -> Result<(), String> {
        if let Some(key) = self.resolve_target_key(envelope, true).await {
            self.deliver_resolved(key, envelope).await?;
        }
        Ok(())
    }
}

fn attach_auth_routes(r: Router) -> Router {
    r.route("/dashboard", get(assets_http::handle_dashboard_page))
        .route("/dashboard/login", get(assets_http::handle_login_page))
        .route("/dashboard/magic/:token", get(auth_http::handle_magic_exchange))
        .route("/dashboard/manifest.webmanifest", get(assets_http::handle_manifest))
        .route("/dashboard/sw.js", get(assets_http::handle_sw))
        .route("/dashboard/icon.svg", get(assets_http::handle_icon))
        .route("/dashboard/assets/:file", get(assets_http::handle_asset))
        .route("/dashboard/api/auth/me", get(auth_http::handle_auth_me))
        .route("/dashboard/api/auth/login", post(auth_http::handle_login))
        .route("/dashboard/api/auth/pin", post(auth_http::handle_set_pin))
        .route("/dashboard/api/auth/logout", post(auth_http::handle_logout))
        .route("/dashboard/api/auth/revoke_all", post(auth_http::handle_revoke_all))
        .route("/dashboard/api/ws", get(ws::handle_ws))
}

fn attach_session_routes(r: Router) -> Router {
    r.route(
        "/dashboard/api/sessions",
        get(api_sessions::handle_list_sessions)
            .post(api_sessions::handle_create_session)
            .delete(api_chat::handle_delete_session),
    )
        .route("/dashboard/api/sessions/config", patch(api_sessions::handle_patch_session_config))
        .route("/dashboard/api/sessions/history", get(api_history::handle_session_history))
        .route("/dashboard/api/sessions/upload", post(api_history::handle_upload_file).layer(DefaultBodyLimit::max(32 * 1024 * 1024)))
        .route("/dashboard/api/sessions/send", post(api_chat::handle_send_prompt))
        .route("/dashboard/api/sessions/ask", post(api_ask::handle_answer_ask))
        .route("/dashboard/api/sessions/stop", post(api_chat::handle_stop_session))
        .route("/dashboard/api/sessions/new", post(api_chat::handle_new_session))
        .route("/dashboard/api/sessions/prune", post(api_chat::handle_prune_sessions))
        .route("/dashboard/api/models", get(api_models::handle_list_models))
}

fn attach_ops_routes(r: Router) -> Router {
    r.route("/dashboard/api/subagents", get(subagent_tree::handle_get_subagents))
        .route("/dashboard/api/usage", get(api_ops::handle_get_usage))
        .route("/dashboard/api/cron", get(api_ops::handle_list_cron))
        .route("/dashboard/api/cron/:id/toggle", post(api_ops::handle_toggle_cron))
        .route("/dashboard/api/cron/:id/run", post(api_ops::handle_run_cron))
        .route("/dashboard/api/file", get(api_files::handle_get_file))
        .route("/dashboard/api/vault/entries", get(api_files::handle_vault_list).post(api_files::handle_vault_upsert))
        .route("/dashboard/api/vault/entries/:site_key", delete(api_files::handle_vault_delete))
        .route("/dashboard/api/hitl", get(api_hitl::handle_hitl_status))
        .route("/dashboard/api/hitl/start", post(api_hitl::handle_hitl_start))
}

/// Builds the `/dashboard` and `/dashboard/api/*` Axum router.
pub fn dashboard_routes() -> Router {
    attach_ops_routes(attach_session_routes(attach_auth_routes(Router::new())))
}

/// Builds an isolated `/dashboard` router with an explicit [`WebAppCtx`] layer (for tests).
pub fn build_router_with_ctx(ctx: Arc<WebAppCtx>) -> Router {
    dashboard_routes().layer(Extension(ctx))
}
