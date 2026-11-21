//! # Batch-Aware & Turn-Safe Voice Briefing Coordinator (`voice_briefing.rs`)
//!
//! ## Overview
//! Coordinates background [`WebEvent`] notifications for an active Gemini Live voice session:
//! 1. **Zero Mid-Sentence Self-Interruption**: Queues background events while the assistant is
//!    actively speaking (`AudioPcm24`/`BotTranscript` until `TurnComplete` + playback drain).
//! 2. **Parallel Batch Synthesis**: Holds early completions while sibling delegated topics are
//!    still running (`busy && !asking`) so parallel tasks report back as one unified synthesis.
//!
//! ## Search Tags
//! #voice-briefing, #parallel-synthesis, #batch-completion, #barge-guard

use super::api_sessions::check_busy_and_asking;
use super::auth_http::{json_err, require_auth};
use super::events::WebEvent;
use super::voice_protocol::UpstreamAction;
use super::voice_schema::{resolve_gemini_api_key, resolve_voice_model};
use super::ws::is_valid_ws_origin;
use super::{resolve_ctx, WebAppCtx};
use crate::security::vault_auth::proxy::constant_time_eq;
use crate::session::key::SessionKey;
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Query parameters for `GET /dashboard/api/voice/ws`.
#[derive(Deserialize)]
pub struct VoiceWsQuery {
    /// Session CSRF token.
    #[serde(default)]
    pub csrf: String,
    /// Interface language (`ko`, `en`, or `auto`).
    #[serde(default)]
    pub lang: String,
    /// Optional currently viewed session key.
    #[serde(default)]
    pub topic: String,
}

/// Returns Voice Desk Assistant readiness (`GET /dashboard/api/voice/status`).
pub async fn handle_voice_status(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    let sess = match require_auth(&ctx, &headers).await {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let ready = resolve_gemini_api_key(&ctx.tuner_home).is_some();
    Json(json!({
        "ready": ready,
        "model": resolve_voice_model().trim_start_matches("models/"),
        "csrf_token": sess.csrf_token
    }))
    .into_response()
}

/// Validates origin, cookie session, CSRF token, and `GEMINI_API_KEY` before WS upgrade.
pub async fn authorize_voice_ws(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: &HeaderMap,
    q: &VoiceWsQuery,
) -> Result<(Arc<WebAppCtx>, String, String), Box<Response>> {
    let ctx = resolve_ctx(ext)?;
    if !is_valid_ws_origin(headers) {
        return Err(Box::new(json_err(StatusCode::FORBIDDEN, "WebSocket origin mismatch")));
    }
    let sess = require_auth(&ctx, headers).await?;
    if q.csrf.is_empty() || !constant_time_eq(q.csrf.as_bytes(), sess.csrf_token.as_bytes()) {
        return Err(Box::new(json_err(StatusCode::FORBIDDEN, "Invalid WebSocket CSRF token")));
    }
    let Some(api_key) = resolve_gemini_api_key(&ctx.tuner_home) else {
        return Err(Box::new(json_err(StatusCode::PRECONDITION_FAILED, "GEMINI_API_KEY is not configured")));
    };
    Ok((ctx, sess.sid, api_key))
}

struct CompletedItem {
    session_key: String,
    title: String,
    is_error: bool,
    snippet: String,
    completed_at: Instant,
}

/// Per-connection state coordinating non-interrupting, batch-synthesized voice briefings.
pub struct VoiceBriefingState {
    tracked_topics: HashSet<String>,
    assistant_speaking: bool,
    last_activity_at: Instant,
    pending_asks: Vec<String>,
    pending_results: Vec<CompletedItem>,
}

impl Default for VoiceBriefingState {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceBriefingState {
    /// Creates an idle [`VoiceBriefingState`].
    pub fn new() -> Self {
        Self {
            tracked_topics: HashSet::new(),
            assistant_speaking: false,
            last_activity_at: Instant::now() - Duration::from_secs(10),
            pending_asks: Vec::new(),
            pending_results: Vec::new(),
        }
    }

    /// Registers a topic `session_key` delegated during this voice session for batch tracking.
    pub fn record_delegated(&mut self, session_key: &str) {
        if !session_key.trim().is_empty() {
            self.tracked_topics.insert(session_key.trim().to_string());
        }
    }

    /// Updates live assistant speaking state from upstream Gemini Live frames.
    pub fn on_upstream_action(&mut self, act: &UpstreamAction) {
        match act {
            UpstreamAction::AudioPcm24(_)
            | UpstreamAction::BotTranscript(_)
            | UpstreamAction::ToolCalls(_) => {
                self.assistant_speaking = true;
                self.last_activity_at = Instant::now();
            }
            UpstreamAction::TurnComplete | UpstreamAction::Interrupted => {
                self.assistant_speaking = false;
                self.last_activity_at = Instant::now();
            }
            _ => {}
        }
    }

    /// Resets transient speaking lock when reconnecting upstream.
    pub fn on_upstream_reconnect(&mut self) {
        self.assistant_speaking = false;
    }

    fn can_inject_now(&self) -> bool {
        if self.assistant_speaking {
            return self.last_activity_at.elapsed() > Duration::from_secs(20);
        }
        self.last_activity_at.elapsed() >= Duration::from_millis(900)
    }

    /// Buffers a relevant background [`WebEvent`] (`AskQuestion` or `TurnResult`).
    pub async fn ingest_web_event(&mut self, ctx: &WebAppCtx, ev: &WebEvent) {
        match ev {
            WebEvent::AskQuestion {
                session_key,
                questions,
                current_index,
                ..
            } => {
                let Some(q) = questions.get(*current_index) else { return };
                let title = resolve_topic_title(ctx, session_key).await;
                let opts = q
                    .options
                    .iter()
                    .enumerate()
                    .map(|(i, o)| format!("{}. {}", i + 1, o))
                    .collect::<Vec<_>>()
                    .join(" / ");
                self.pending_asks.push(format!(
                    "[BACKGROUND_EVENT] Topic \"{title}\" (`{session_key}`) is waiting on AskQuestion ({}/{}): \"{}\" Options: [{}]. Read this question and options clearly to the user so they can answer by voice.",
                    current_index + 1,
                    questions.len(),
                    q.question,
                    opts
                ));
            }
            WebEvent::TurnResult {
                session_key,
                text,
                is_error,
                ..
            } => {
                let title = resolve_topic_title(ctx, session_key).await;
                let snippet: String = text.chars().take(1100).collect();
                self.pending_results.retain(|x| x.session_key != *session_key);
                self.pending_results.push(CompletedItem {
                    session_key: session_key.clone(),
                    title,
                    is_error: *is_error,
                    snippet,
                    completed_at: Instant::now(),
                });
            }
            _ => {}
        }
    }

    async fn collect_busy_tracked_siblings(&self, ctx: &WebAppCtx) -> Vec<String> {
        let mut busy_titles = Vec::new();
        for sk in &self.tracked_topics {
            if self.pending_results.iter().any(|r| &r.session_key == sk) {
                continue;
            }
            let Ok(key) = sk.parse::<SessionKey>() else { continue };
            let Ok(Some(sess)) = ctx.sessions.get_active(&key).await else { continue };
            let (busy, asking) = check_busy_and_asking(ctx, &sess).await;
            if busy && !asking {
                let t = sess.topic_name.unwrap_or_else(|| sk.clone());
                busy_titles.push(t);
            }
        }
        busy_titles
    }

    /// Returns a synthesized briefing string when safe to speak and all parallel siblings settled.
    pub async fn poll_ready_briefing(&mut self, ctx: &WebAppCtx) -> Option<String> {
        if !self.can_inject_now() {
            return None;
        }
        if !self.pending_asks.is_empty() {
            let joined = self.pending_asks.drain(..).collect::<Vec<_>>().join("\n\n");
            self.assistant_speaking = true;
            self.last_activity_at = Instant::now();
            return Some(joined);
        }
        if self.pending_results.is_empty() {
            return None;
        }
        let newest = self.pending_results.iter().map(|i| i.completed_at).max()?;
        if newest.elapsed() < Duration::from_millis(550) {
            return None;
        }
        let oldest = self.pending_results.iter().map(|i| i.completed_at).min()?;
        let busy_siblings = self.collect_busy_tracked_siblings(ctx).await;
        if !busy_siblings.is_empty() && oldest.elapsed() < Duration::from_secs(75) {
            return None;
        }
        let items: Vec<CompletedItem> = self.pending_results.drain(..).collect();
        for it in &items {
            self.tracked_topics.remove(&it.session_key);
        }
        self.assistant_speaking = true;
        self.last_activity_at = Instant::now();
        Some(format_synthesized_briefing(&items, &busy_siblings))
    }
}

async fn resolve_topic_title(ctx: &WebAppCtx, session_key: &str) -> String {
    if let Ok(k) = session_key.parse::<SessionKey>()
        && let Ok(Some(s)) = ctx.sessions.get_active(&k).await
        && let Some(t) = s.topic_name.filter(|n| !n.trim().is_empty())
    {
        return t;
    }
    session_key.to_string()
}

fn format_synthesized_briefing(items: &[CompletedItem], still_busy: &[String]) -> String {
    let busy_note = if still_busy.is_empty() {
        String::new()
    } else {
        format!("\n(Note: Still running in background: {})", still_busy.join(", "))
    };
    if items.len() == 1 && still_busy.is_empty() {
        let it = &items[0];
        let st = if it.is_error { "ERROR" } else { "COMPLETED" };
        return format!(
            "[BACKGROUND_EVENT] Topic \"{}\" (`{}`) {st}.{busy_note}\nResult excerpt:\n{}\n\
             Briefing rule: Do NOT read raw Markdown tables or long lists verbatim. Summarize the core takeaway in 2-3 natural spoken sentences aligned with the user's request.",
            it.title, it.session_key, it.snippet
        );
    }
    let mut blocks = Vec::with_capacity(items.len());
    for (idx, it) in items.iter().enumerate() {
        let st = if it.is_error { "ERROR" } else { "COMPLETED" };
        blocks.push(format!(
            "--- [{}] \"{}\" (`{}`) [{}] ---\n{}",
            idx + 1,
            it.title,
            it.session_key,
            st,
            it.snippet
        ));
    }
    format!(
        "[BATCH_COMPLETION_BRIEFING] {} parallel topics finished:{busy_note}\n\n{}\n\n\
         CRITICAL BRIEFING INSTRUCTION: Do NOT report each topic one-by-one or read raw Markdown aloud! \
         Synthesize all {} results together according to the user's overarching goal (highlighting key comparisons, conclusions, and actionable differences) in 3-5 concise, natural spoken sentences.",
        items.len(),
        blocks.join("\n\n"),
        items.len()
    )
}
