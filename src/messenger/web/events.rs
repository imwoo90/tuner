//! # Web Dashboard Real-Time Event Broadcast Hub (`events.rs`)
//!
//! ## Overview
//! Distributes live agent streaming deltas, user prompts, interactive question state,
//! turn completions, and cron status updates over a shared broadcast channel.
//!
//! ## Search Tags
//! #web-events, #websocket-broadcast, #live-stream-buffer
//!
//! ## Submodules
//! None (leaf module).

use crate::cli::AskQuestionData;
use crate::session::{data::SessionData, key::SessionKey};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, Mutex};

/// Real-time event variants pushed over `/dashboard/api/ws`.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebEvent {
    /// Topic metadata, provider/model/effort, or busy/asking state changed.
    SessionUpdated {
        /// Storage key (`tg:<cid>[:<tid>]` or `web:0:<tid>`).
        session_key: String,
        /// Active provider (`antigravity`, `claude`, `codex`).
        provider: String,
        /// Active canonical model identifier.
        model: String,
        /// Active reasoning effort level, if applicable.
        effort: Option<String>,
        /// Human-readable topic title.
        topic_name: Option<String>,
        /// True when pinned at the top of the sidebar.
        pinned: bool,
        /// True while a foreground or background turn is active.
        busy: bool,
        /// True while waiting for an `AskQuestion` response.
        asking: bool,
    },
    /// User prompt accepted from Telegram or Web UI.
    UserMessage {
        /// Storage key of the target session.
        session_key: String,
        /// Originating transport (`tg` or `web`).
        source: String,
        /// Raw message text.
        text: String,
        /// RFC3339 timestamp.
        timestamp: String,
    },
    /// Incremental streaming text/thinking/tool preview delta.
    StreamDelta {
        /// Storage key of the target session.
        session_key: String,
        /// Accumulated streaming markdown preview.
        text: String,
        /// Provider session identifier, if known.
        session_id: Option<String>,
    },
    /// Active interactive `AskQuestion` prompt awaiting user response.
    AskQuestion {
        /// Storage key of the target session.
        session_key: String,
        /// Provider session identifier.
        session_id: String,
        /// Active CLI provider name.
        provider: String,
        /// Ordered questions list.
        questions: Vec<AskQuestionData>,
        /// Zero-based active question index.
        current_index: usize,
        /// Collected answers so far.
        answers: Vec<String>,
    },
    /// Interactive `AskQuestion` flow finished or cancelled.
    AskResolved {
        /// Storage key of the target session.
        session_key: String,
        /// Provider session identifier.
        session_id: String,
        /// Summary of chosen answer or cancellation.
        summary: String,
    },
    /// Final turn output persisted and ready in chat history.
    TurnResult {
        /// Storage key of the target session.
        session_key: String,
        /// Provider session identifier, if any.
        session_id: Option<String>,
        /// Final rendered text or formatted error string.
        text: String,
        /// True if the turn ended with an error.
        is_error: bool,
        /// Safe workspace file paths referenced in the output.
        files: Vec<String>,
        /// RFC3339 timestamp.
        timestamp: String,
    },
    /// Cron job enabled state or run status changed.
    CronUpdated {
        /// Cron job identifier.
        job_id: String,
        /// Updated enabled flag.
        enabled: bool,
        /// Latest run status string, if any.
        last_run_status: Option<String>,
    },
    /// Active HITL remote browser auth sessions changed.
    HitlUpdated {
        /// True when one or more `/auth/:token` routes are live.
        active: bool,
    },
    /// CLI provider installation or authentication readiness changed.
    ProviderSetupUpdated {
        /// Provider identifier (`antigravity`, `claude`, `codex`).
        provider: String,
    },
}

struct LiveBuffer {
    text: String,
    last_emit: Instant,
}

/// Broadcast hub and in-flight streaming buffer registry for the Web Dashboard.
pub struct WebEventHub {
    tx: broadcast::Sender<WebEvent>,
    live_buffers: Mutex<HashMap<String, LiveBuffer>>,
    delta_throttle: Mutex<HashMap<String, Instant>>,
}

impl Default for WebEventHub {
    fn default() -> Self {
        Self::new()
    }
}

impl WebEventHub {
    /// Creates a new `WebEventHub` with a 512-message broadcast ring buffer.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(512);
        Self {
            tx,
            live_buffers: Mutex::new(HashMap::new()),
            delta_throttle: Mutex::new(HashMap::new()),
        }
    }

    /// Subscribes a WebSocket client to live [`WebEvent`] notifications.
    pub fn subscribe(&self) -> broadcast::Receiver<WebEvent> {
        self.tx.subscribe()
    }

    /// Publishes an event, updating live stream buffers before broadcasting.
    pub async fn publish(&self, ev: WebEvent) {
        match &ev {
            WebEvent::StreamDelta { session_key, text, .. } => {
                let mut bufs = self.live_buffers.lock().await;
                bufs.insert(session_key.clone(), LiveBuffer { text: text.clone(), last_emit: Instant::now() });
            }
            WebEvent::TurnResult { session_key, .. }
            | WebEvent::SessionUpdated { session_key, busy: false, .. } => {
                self.live_buffers.lock().await.remove(session_key);
                self.delta_throttle.lock().await.remove(session_key);
            }
            _ => {}
        }
        let _ = self.tx.send(ev);
    }

    /// Publishes `StreamDelta` throttled to at most once per `min_interval_ms` per session.
    pub async fn publish_throttled_delta(
        &self,
        session_key: &str,
        text: &str,
        session_id: Option<String>,
        min_interval_ms: u64,
    ) {
        let now = Instant::now();
        {
            let mut bufs = self.live_buffers.lock().await;
            bufs.insert(session_key.to_string(), LiveBuffer { text: text.to_string(), last_emit: now });
        }
        let should_send = {
            let mut thr = self.delta_throttle.lock().await;
            let ready = thr.get(session_key).is_none_or(|t| t.elapsed() >= Duration::from_millis(min_interval_ms));
            if ready {
                thr.insert(session_key.to_string(), now);
            }
            ready
        };
        if should_send {
            let _ = self.tx.send(WebEvent::StreamDelta {
                session_key: session_key.to_string(),
                text: text.to_string(),
                session_id,
            });
        }
    }

    /// Returns any currently active streaming text buffer for `session_key`.
    pub async fn get_live_stream(&self, session_key: &str) -> Option<String> {
        let mut bufs = self.live_buffers.lock().await;
        bufs.retain(|_, b| b.last_emit.elapsed() < Duration::from_secs(600));
        bufs.get(session_key).map(|b| b.text.clone())
    }
}

/// Emits an event to the global [`super::WebAppCtx`] event hub when initialized.
pub async fn publish_global(ev: WebEvent) {
    if let Some(ctx) = super::global_web_ctx() {
        ctx.events.publish(ev).await;
    }
}

/// Emits a throttled streaming delta to the global event hub when initialized.
pub async fn publish_global_delta(session_key: &str, text: &str, session_id: Option<String>) {
    if let Some(ctx) = super::global_web_ctx() {
        ctx.events.publish_throttled_delta(session_key, text, session_id, 100).await;
    }
}

/// Helper to emit a [`WebEvent::SessionUpdated`] snapshot from a [`SessionData`].
pub async fn emit_session_updated(ctx: &Arc<super::WebAppCtx>, sess: &SessionData, busy: bool, asking: bool) {
    let (prov, model, effort) = SessionData::resolve_effective_target(Some(sess), &ctx.config);
    ctx.events
        .publish(WebEvent::SessionUpdated {
            session_key: sess.session_key().storage_key(),
            provider: prov,
            model,
            effort,
            topic_name: sess.topic_name.clone(),
            pinned: sess.pinned,
            busy,
            asking,
        })
        .await;
}

/// Helper to emit [`WebEvent::SessionUpdated`] via global context when available.
pub async fn publish_session_snapshot(sess: &SessionData, busy: bool, asking: bool) {
    if let Some(ctx) = super::global_web_ctx() {
        emit_session_updated(&ctx, sess, busy, asking).await;
    }
}

/// Emits a mirrored Telegram user prompt + `busy=true` snapshot to connected Web clients.
pub async fn emit_tg_user_message(sess: &SessionData, text: &str) {
    if let Some(ctx) = super::global_web_ctx() {
        ctx.events
            .publish(WebEvent::UserMessage {
                session_key: sess.session_key().storage_key(),
                source: "tg".to_string(),
                text: text.to_string(),
                timestamp: chrono::Utc::now().to_rfc3339(),
            })
            .await;
        emit_session_updated(&ctx, sess, true, false).await;
    }
}

/// Emits a mirrored Telegram turn completion + safe preview file paths to Web clients.
pub async fn emit_tg_turn_result(
    cid: i64,
    tid: Option<i64>,
    sid: Option<String>,
    text: String,
    is_error: bool,
) {
    let Some(ctx) = super::global_web_ctx() else { return };
    let sk = SessionKey::telegram(cid, tid).storage_key();
    let files = super::api_files::filter_safe_preview_paths(&ctx, &text);
    ctx.events
        .publish(WebEvent::TurnResult {
            session_key: sk,
            session_id: sid,
            text,
            is_error,
            files,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
        .await;
    if let Ok(Some(sess)) = ctx.sessions.get_active(&SessionKey::telegram(cid, tid)).await {
        let (_, asking) = super::api_sessions::check_busy_and_asking(&ctx, &sess).await;
        emit_session_updated(&ctx, &sess, asking, asking).await;
    }
}

/// Emits a mirrored Telegram `AskQuestion` card state to Web clients.
pub async fn emit_tg_ask_question(
    sess: &SessionData,
    sid: &str,
    prov: &str,
    state: &crate::cli::antigravity::session::AskState,
) {
    publish_global(WebEvent::AskQuestion {
        session_key: sess.session_key().storage_key(),
        session_id: sid.to_string(),
        provider: prov.to_string(),
        questions: state.questions.clone(),
        current_index: state.current_index,
        answers: state.answers.clone(),
    })
    .await;
    publish_session_snapshot(sess, true, true).await;
}

/// Emits a mirrored Telegram `AskQuestion` resolution event to Web clients.
pub async fn emit_tg_ask_resolved(cid: i64, tid: Option<i64>, sid: &str, summary: &str) {
    publish_global(WebEvent::AskResolved {
        session_key: SessionKey::telegram(cid, tid).storage_key(),
        session_id: sid.to_string(),
        summary: summary.to_string(),
    })
    .await;
}

/// Spawns non-blocking `CronUpdated` event broadcast for cron state changes.
pub fn spawn_cron_updated(job_id: &str, enabled: bool, last_run_status: Option<String>) {
    let jid = job_id.to_string();
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::spawn(async move {
            publish_global(WebEvent::CronUpdated { job_id: jid, enabled, last_run_status }).await;
        });
    }
}
