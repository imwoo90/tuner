//! # Real-Time Voice Desk Assistant WebSocket Bridge (`voice_ws.rs`)
//!
//! ## Overview
//! Authenticates `/dashboard/api/voice/ws` and bridges browser 16 kHz PCM microphone audio
//! and 24 kHz PCM speech output with Gemini Live (`BidiGenerateContent`), executing front-desk
//! tools server-side without ever exposing `GEMINI_API_KEY` to browser JavaScript.
//!
//! ## Search Tags
//! #voice-websocket, #gemini-live-bridge, #desk-assistant, #session-resumption

use super::events::WebEvent;
pub use super::voice_briefing::{handle_voice_status, VoiceWsQuery};
use super::voice_briefing::{authorize_voice_ws, VoiceBriefingState};
use super::voice_protocol::{
    action_to_client_frame, build_live_setup_frame, parse_gemini_server_frame, UpstreamAction,
};
use super::voice_schema::{build_upstream_url, translate_browser_msg};
use super::voice_tools::execute_voice_tool;
use super::WebAppCtx;
use axum::{
    extract::{
        ws::{Message as AxumWsMsg, WebSocket, WebSocketUpgrade},
        Query,
    },
    http::HeaderMap,
    response::Response,
    Extension,
};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio_tungstenite::tungstenite::{Error as TungErr, Message as TungMsg};

type ClientSink = futures::stream::SplitSink<WebSocket, AxumWsMsg>;
type ClientStream = futures::stream::SplitStream<WebSocket>;
type LegState<'a> = (
    &'a mut Option<String>,
    &'a mut Option<String>,
    &'a mut bool,
    &'a mut VoiceBriefingState,
);

/// Upgrades `/dashboard/api/voice/ws` to a real-time bidirectional Gemini Live session.
pub async fn handle_voice_ws(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Query(q): Query<VoiceWsQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    match authorize_voice_ws(ext, &headers, &q).await {
        Ok((ctx, sid, api_key)) => ws.on_upgrade(move |socket| run_voice_bridge(socket, ctx, sid, api_key, q)),
        Err(resp) => *resp,
    }
}

async fn send_client_json(client_tx: &mut ClientSink, payload: Value) -> bool {
    client_tx.send(AxumWsMsg::Text(payload.to_string())).await.is_ok()
}

async fn run_tool_calls_and_respond<S: futures::Sink<TungMsg> + Unpin>(
    calls: Vec<(String, String, Value)>,
    ctx: &Arc<WebAppCtx>,
    current_topic: &mut Option<String>,
    briefing: &mut VoiceBriefingState,
    client_tx: &mut ClientSink,
    up_tx: &mut S,
) -> bool {
    let mut responses = Vec::with_capacity(calls.len());
    for (id, name, args) in calls {
        let _ = send_client_json(client_tx, json!({ "type": "tool_call", "name": name })).await;
        let (output, focus) = execute_voice_tool(ctx, current_topic, &name, &args).await;
        if let Some(sk) = focus {
            if name == "delegate_to_topic" {
                briefing.record_delegated(&sk);
            }
            let _ = send_client_json(client_tx, json!({ "type": "focus_topic", "session_key": sk })).await;
        }
        responses.push(json!({ "id": id, "name": name, "response": { "output": output } }));
    }
    let tr = json!({ "toolResponse": { "functionResponses": responses } }).to_string();
    up_tx.send(TungMsg::Text(tr)).await.is_err()
}

async fn handle_upstream_actions<S: futures::Sink<TungMsg> + Unpin>(
    actions: Vec<UpstreamAction>,
    ctx: &Arc<WebAppCtx>,
    state: LegState<'_>,
    client_tx: &mut ClientSink,
    up_tx: &mut S,
) -> Result<bool, ()> {
    let (current_topic, resume_handle, got_ready, briefing) = state;
    for act in actions {
        briefing.on_upstream_action(&act);
        if matches!(act, UpstreamAction::Ready) {
            *got_ready = true;
        }
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
                if run_tool_calls_and_respond(calls, ctx, current_topic, briefing, client_tx, up_tx).await {
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
    Reconnect(bool),
    Stop,
}

fn exit_for_drop(state: &LegState<'_>) -> BridgeExit {
    if state.1.is_some() { BridgeExit::Reconnect(*state.2) } else { BridgeExit::Stop }
}

async fn dispatch_raw_upstream<S: futures::Sink<TungMsg> + Unpin>(
    umsg: Option<Result<TungMsg, TungErr>>,
    ctx: &Arc<WebAppCtx>,
    state: LegState<'_>,
    client_tx: &mut ClientSink,
    up_tx: &mut S,
) -> Option<BridgeExit> {
    let raw = match umsg {
        Some(Ok(TungMsg::Text(s))) => s,
        Some(Ok(TungMsg::Binary(b))) => String::from_utf8(b).ok()?,
        Some(Ok(TungMsg::Ping(p))) => {
            let _ = up_tx.send(TungMsg::Pong(p)).await;
            return None;
        }
        Some(Ok(TungMsg::Close(_))) | Some(Err(_)) | None => return Some(exit_for_drop(&state)),
        _ => return None,
    };
    let has_res = state.1.is_some();
    match handle_upstream_actions(parse_gemini_server_frame(&raw), ctx, state, client_tx, up_tx).await {
        Ok(true) => Some(if has_res { BridgeExit::Reconnect(true) } else { BridgeExit::Stop }),
        Ok(false) => None,
        Err(()) => Some(BridgeExit::ClientClosed),
    }
}

async fn dispatch_client_frame<S: futures::Sink<TungMsg> + Unpin>(
    cmsg: Option<Result<AxumWsMsg, axum::Error>>,
    current_topic: &mut Option<String>,
    reconn: (bool, bool),
    client_tx: &mut ClientSink,
    up_tx: &mut S,
) -> Option<BridgeExit> {
    match cmsg {
        Some(Ok(AxumWsMsg::Text(raw))) => {
            if let Some(fwd) = translate_browser_msg(&raw, current_topic)
                && up_tx.send(TungMsg::Text(fwd)).await.is_err()
            {
                return Some(if reconn.0 { BridgeExit::Reconnect(reconn.1) } else { BridgeExit::Stop });
            }
            None
        }
        Some(Ok(AxumWsMsg::Ping(p))) => {
            let _ = client_tx.send(AxumWsMsg::Pong(p)).await;
            None
        }
        Some(Ok(AxumWsMsg::Close(_))) | None => Some(BridgeExit::ClientClosed),
        _ => None,
    }
}

async fn pump_single_upstream<S, R>(
    ctx: &Arc<WebAppCtx>,
    client_tx: &mut ClientSink,
    client_rx: &mut ClientStream,
    events_rx: &mut tokio::sync::broadcast::Receiver<WebEvent>,
    up_tx: &mut S,
    up_rx: &mut R,
    state: (&mut Option<String>, &mut Option<String>, &mut VoiceBriefingState),
) -> BridgeExit
where
    S: futures::Sink<TungMsg> + Unpin,
    R: futures::Stream<Item = Result<TungMsg, TungErr>> + Unpin,
{
    let (current_topic, resume_handle, briefing) = state;
    briefing.on_upstream_reconnect();
    let mut got_ready = false;
    let mut ka = tokio::time::interval(std::time::Duration::from_secs(20));
    let mut brief_tick = tokio::time::interval(std::time::Duration::from_millis(400));
    ka.tick().await;
    loop {
        tokio::select! {
            _ = ka.tick() => {
                let _ = client_tx.send(AxumWsMsg::Ping(Vec::new())).await;
                let _ = up_tx.send(TungMsg::Ping(Vec::new())).await;
            }
            _ = brief_tick.tick() => if got_ready
                && let Some(note) = briefing.poll_ready_briefing(ctx).await
            {
                let _ = up_tx.send(TungMsg::Text(json!({ "realtimeInput": { "text": note } }).to_string())).await;
            },
            cmsg = client_rx.next() => {
                let rc = (resume_handle.is_some(), got_ready);
                if let Some(ex) = dispatch_client_frame(cmsg, current_topic, rc, client_tx, up_tx).await {
                    return ex;
                }
            },
            umsg = up_rx.next() => {
                let ls = (&mut *current_topic, &mut *resume_handle, &mut got_ready, &mut *briefing);
                if let Some(ex) = dispatch_raw_upstream(umsg, ctx, ls, client_tx, up_tx).await {
                    return ex;
                }
            },
            ev = events_rx.recv() => if let Ok(web_ev) = ev {
                briefing.ingest_web_event(ctx, &web_ev).await;
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
    let mut briefing = VoiceBriefingState::new();
    let mut bad_legs = 0u8;
    let lang = if q.lang.trim().is_empty() { "ko".to_string() } else { q.lang };

    while ctx.auth.verify_session(&sid).await.is_some() {
        let Ok((up_ws, _)) = tokio_tungstenite::connect_async(build_upstream_url(&api_key)).await else {
            let _ = send_client_json(&mut client_tx, json!({ "type": "error", "error": "Failed to connect to Gemini Live API" })).await;
            break;
        };
        let (mut up_tx, mut up_rx) = up_ws.split();
        let setup = build_live_setup_frame(&lang, current_topic.as_deref(), resume_handle.as_deref());
        if up_tx.send(TungMsg::Text(setup)).await.is_err() {
            break;
        }
        match pump_single_upstream(&ctx, &mut client_tx, &mut client_rx, &mut events_rx, &mut up_tx, &mut up_rx, (&mut current_topic, &mut resume_handle, &mut briefing)).await {
            BridgeExit::Reconnect(true) => bad_legs = 0,
            BridgeExit::Reconnect(false) if bad_legs < 2 => {
                bad_legs += 1;
                resume_handle = None;
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
            _ => break,
        }
    }
    let _ = client_tx.send(AxumWsMsg::Close(None)).await;
}
