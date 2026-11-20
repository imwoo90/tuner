//! # Real-Time Voice Desk Assistant WebSocket Bridge (`voice_ws.rs`)
//!
//! ## Overview
//! Authenticates `/dashboard/api/voice/ws` and bridges browser 16 kHz PCM microphone audio
//! and 24 kHz PCM speech output with Gemini Live (`BidiGenerateContent`), executing front-desk
//! tools server-side without ever exposing `GEMINI_API_KEY` to browser JavaScript.
//!
//! ## Search Tags
//! #voice-websocket, #gemini-live-bridge, #desk-assistant, #session-resumption

use super::auth_http::{json_err, require_auth};
use super::events::WebEvent;
use super::voice_protocol::{
    action_to_client_frame, build_live_setup_frame, format_background_web_event,
    parse_gemini_server_frame, resolve_gemini_api_key, resolve_voice_model, UpstreamAction,
};
use super::voice_schema::{build_upstream_url, translate_browser_msg};
use super::voice_tools::execute_voice_tool;
use super::ws::is_valid_ws_origin;
use super::{resolve_ctx, WebAppCtx};
use crate::security::vault_auth::proxy::constant_time_eq;
use axum::{
    extract::{
        ws::{Message as AxumWsMsg, WebSocket, WebSocketUpgrade},
        Query,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio_tungstenite::tungstenite::Message as TungMsg;

/// Query parameters for `GET /dashboard/api/voice/ws`.
#[derive(Deserialize)]
pub struct VoiceWsQuery {
    /// Session CSRF token.
    #[serde(default)]
    pub csrf: String,
    /// Interface language (`ko` or `en`).
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
    if let Err(r) = require_auth(&ctx, &headers).await {
        return *r;
    }
    let ready = resolve_gemini_api_key(&ctx.tuner_home).is_some();
    Json(json!({
        "ready": ready,
        "model": resolve_voice_model().trim_start_matches("models/")
    }))
    .into_response()
}

/// Upgrades `/dashboard/api/voice/ws` to a real-time bidirectional Gemini Live session.
pub async fn handle_voice_ws(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Query(q): Query<VoiceWsQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if !is_valid_ws_origin(&headers) {
        return json_err(StatusCode::FORBIDDEN, "WebSocket origin mismatch");
    }
    let sess = match require_auth(&ctx, &headers).await {
        Ok(s) => s,
        Err(r) => return *r,
    };
    if q.csrf.is_empty() || !constant_time_eq(q.csrf.as_bytes(), sess.csrf_token.as_bytes()) {
        return json_err(StatusCode::FORBIDDEN, "Invalid WebSocket CSRF token");
    }
    let Some(api_key) = resolve_gemini_api_key(&ctx.tuner_home) else {
        return json_err(StatusCode::PRECONDITION_FAILED, "GEMINI_API_KEY is not configured");
    };
    let sid = sess.sid;
    ws.on_upgrade(move |socket| run_voice_bridge(socket, ctx, sid, api_key, q))
}

async fn send_client_json(
    client_tx: &mut futures::stream::SplitSink<WebSocket, AxumWsMsg>,
    payload: Value,
) -> bool {
    client_tx.send(AxumWsMsg::Text(payload.to_string())).await.is_ok()
}

async fn run_tool_calls_and_respond<S>(
    calls: Vec<(String, String, Value)>,
    ctx: &Arc<WebAppCtx>,
    current_topic: &mut Option<String>,
    client_tx: &mut futures::stream::SplitSink<WebSocket, AxumWsMsg>,
    up_tx: &mut S,
) -> bool
where
    S: futures::Sink<TungMsg> + Unpin,
{
    let mut responses = Vec::with_capacity(calls.len());
    for (id, name, args) in calls {
        let _ = send_client_json(client_tx, json!({ "type": "tool_call", "name": name })).await;
        let (output, focus) = execute_voice_tool(ctx, current_topic, &name, &args).await;
        if let Some(sk) = focus {
            let _ = send_client_json(client_tx, json!({ "type": "focus_topic", "session_key": sk })).await;
        }
        responses.push(json!({ "id": id, "name": name, "response": { "output": output } }));
    }
    let tr = json!({ "toolResponse": { "functionResponses": responses } }).to_string();
    up_tx.send(TungMsg::Text(tr)).await.is_err()
}

async fn handle_upstream_actions<S>(
    actions: Vec<UpstreamAction>,
    ctx: &Arc<WebAppCtx>,
    current_topic: &mut Option<String>,
    resume_handle: &mut Option<String>,
    client_tx: &mut futures::stream::SplitSink<WebSocket, AxumWsMsg>,
    up_tx: &mut S,
) -> Result<bool, ()>
where
    S: futures::Sink<TungMsg> + Unpin,
{
    for act in actions {
        if let Some(frame) = action_to_client_frame(&act) {
            if !send_client_json(client_tx, frame).await {
                return Err(());
            }
            continue;
        }
        match act {
            UpstreamAction::ResumeHandle(h) => *resume_handle = Some(h),
            UpstreamAction::GoAway => return Ok(true),
            UpstreamAction::ToolCalls(calls) => {
                if run_tool_calls_and_respond(calls, ctx, current_topic, client_tx, up_tx).await {
                    return Ok(true);
                }
            }
            _ => {}
        }
    }
    Ok(false)
}

enum BridgeExit {
    ClientClosed,
    Reconnect,
    Stop,
}

async fn dispatch_raw_upstream<S>(
    umsg: Option<Result<TungMsg, tokio_tungstenite::tungstenite::Error>>,
    ctx: &Arc<WebAppCtx>,
    current_topic: &mut Option<String>,
    resume_handle: &mut Option<String>,
    client_tx: &mut futures::stream::SplitSink<WebSocket, AxumWsMsg>,
    up_tx: &mut S,
) -> Option<BridgeExit>
where
    S: futures::Sink<TungMsg> + Unpin,
{
    let raw = match umsg {
        Some(Ok(TungMsg::Text(s))) => s,
        Some(Ok(TungMsg::Binary(b))) => String::from_utf8(b).ok()?,
        Some(Ok(TungMsg::Ping(p))) => {
            let _ = up_tx.send(TungMsg::Pong(p)).await;
            return None;
        }
        Some(Ok(TungMsg::Close(_))) | None => {
            return Some(if resume_handle.is_some() { BridgeExit::Reconnect } else { BridgeExit::Stop });
        }
        _ => return None,
    };
    match handle_upstream_actions(parse_gemini_server_frame(&raw), ctx, current_topic, resume_handle, client_tx, up_tx).await {
        Ok(true) => Some(if resume_handle.is_some() { BridgeExit::Reconnect } else { BridgeExit::Stop }),
        Ok(false) => None,
        Err(()) => Some(BridgeExit::ClientClosed),
    }
}

async fn pump_single_upstream<S, R>(
    ctx: &Arc<WebAppCtx>,
    client_tx: &mut futures::stream::SplitSink<WebSocket, AxumWsMsg>,
    client_rx: &mut futures::stream::SplitStream<WebSocket>,
    events_rx: &mut tokio::sync::broadcast::Receiver<WebEvent>,
    up_tx: &mut S,
    up_rx: &mut R,
    state: (&mut Option<String>, &mut Option<String>),
) -> BridgeExit
where
    S: futures::Sink<TungMsg> + Unpin,
    R: futures::Stream<Item = Result<TungMsg, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let (current_topic, resume_handle) = state;
    loop {
        tokio::select! {
            cmsg = client_rx.next() => match cmsg {
                Some(Ok(AxumWsMsg::Text(raw))) => {
                    if let Some(fwd) = translate_browser_msg(&raw, current_topic)
                        && up_tx.send(TungMsg::Text(fwd)).await.is_err()
                    {
                        return if resume_handle.is_some() { BridgeExit::Reconnect } else { BridgeExit::Stop };
                    }
                }
                Some(Ok(AxumWsMsg::Ping(p))) => { let _ = client_tx.send(AxumWsMsg::Pong(p)).await; }
                Some(Ok(AxumWsMsg::Close(_))) | None => return BridgeExit::ClientClosed,
                _ => {}
            },
            umsg = up_rx.next() => {
                if let Some(ex) = dispatch_raw_upstream(umsg, ctx, current_topic, resume_handle, client_tx, up_tx).await {
                    return ex;
                }
            },
            ev = events_rx.recv() => if let Ok(web_ev) = ev
                && let Some(note) = format_background_web_event(&web_ev)
            {
                let _ = up_tx.send(TungMsg::Text(json!({ "realtimeInput": { "text": note } }).to_string())).await;
            }
        }
    }
}

async fn run_voice_bridge(
    socket: WebSocket,
    ctx: Arc<WebAppCtx>,
    sid: String,
    api_key: String,
    q: VoiceWsQuery,
) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (mut client_tx, mut client_rx) = socket.split();
    let mut events_rx = ctx.events.subscribe();
    let mut current_topic = (!q.topic.trim().is_empty()).then(|| q.topic.trim().to_string());
    let mut resume_handle: Option<String> = None;
    let lang = if q.lang.trim().is_empty() { "ko".to_string() } else { q.lang };

    loop {
        if ctx.auth.verify_session(&sid).await.is_none() {
            break;
        }
        let Ok((up_ws, _)) = tokio_tungstenite::connect_async(build_upstream_url(&api_key)).await else {
            let _ = send_client_json(&mut client_tx, json!({ "type": "error", "error": "Failed to connect to Gemini Live API" })).await;
            break;
        };
        let (mut up_tx, mut up_rx) = up_ws.split();
        let setup = build_live_setup_frame(&lang, current_topic.as_deref(), resume_handle.as_deref());
        if up_tx.send(TungMsg::Text(setup)).await.is_err() {
            break;
        }
        match pump_single_upstream(&ctx, &mut client_tx, &mut client_rx, &mut events_rx, &mut up_tx, &mut up_rx, (&mut current_topic, &mut resume_handle)).await {
            BridgeExit::Reconnect => continue,
            BridgeExit::ClientClosed | BridgeExit::Stop => break,
        }
    }
    let _ = client_tx.send(AxumWsMsg::Close(None)).await;
}
