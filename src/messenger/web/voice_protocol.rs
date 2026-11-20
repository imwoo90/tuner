//! # Gemini Live (`gemini-3.8-live`) Wire Protocol Codec (`voice_protocol.rs`)
//!
//! ## Overview
//! Encodes `BidiGenerateContent` setup, realtime audio/text inputs, tool responses, and
//! background event briefings, and decodes incoming Gemini Live audio/transcript/tool frames.
//!
//! ## Search Tags
//! #gemini-live-protocol, #bidi-audio, #session-resumption, #voice-codec

use super::auth_http::require_auth;
use super::events::WebEvent;
use super::provider_status::read_dotenv_map;
use super::voice_schema::voice_tool_declarations;
use super::{resolve_ctx, WebAppCtx};
use axum::{
    http::HeaderMap,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

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

/// Resolves the configured `GEMINI_API_KEY` from `$TUNER_HOME/.env` or process environment.
pub fn resolve_gemini_api_key(tuner_home: &Path) -> Option<String> {
    let dotenv = read_dotenv_map(&tuner_home.join(".env"));
    dotenv
        .get("GEMINI_API_KEY")
        .cloned()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("GEMINI_API_KEY").ok().filter(|s| !s.trim().is_empty()))
}

/// Returns the configured Gemini Live model identifier (defaults to `models/gemini-3.8-live`).
pub fn resolve_voice_model() -> String {
    let raw = std::env::var("TUNER_VOICE_MODEL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "models/gemini-3.8-live".to_string());
    if raw.starts_with("models/") {
        raw
    } else {
        format!("models/{raw}")
    }
}

fn build_secretary_prompt(lang: &str, active_topic: Option<&str>) -> String {
    let topic_hint = active_topic
        .filter(|s| !s.is_empty())
        .map(|s| format!("Currently open topic on user's screen: `{s}`."))
        .unwrap_or_else(|| "User is currently on a blank Draft screen (no topic selected yet).".to_string());
    if lang.starts_with("en") {
        format!(
            "You are Woo-Tuner ([우튜너]), the user's real-time Front-Desk Voice Secretary in front of the Tuner multi-agent workspace. {topic_hint}\n\
             Rules:\n\
             1. Speak naturally, concisely, and warmly (1-3 short sentences). Never read Markdown symbols like `**` or backticks aloud.\n\
             2. Answer quick questions and status checks directly (`list_topics`, `get_topic_context`).\n\
             3. Delegate coding, file editing, shell commands, builds, or deep research to background CLI topics via `delegate_to_topic`.\n\
             4. When a `[BACKGROUND_EVENT]` message arrives announcing that a topic finished or asked an `AskQuestion`, immediately brief the user aloud (and read numbered options clearly so they can answer by voice via `answer_topic_question`)."
        )
    } else {
        format!(
            "당신은 사용자의 멀티 에이전트 워크스페이스 앞단을 지키는 총괄 음성 비서 '우튜너'입니다. {topic_hint}\n\
             수칙:\n\
             1. 실제 음성 통화처럼 자연스럽고 간결하게(1~3문장 이내) 한국어로 말하세요. 별표(**)나 백틱 같은 마크다운 기호는 절대 소리 내어 읽지 마세요.\n\
             2. 가벼운 질문이나 현황 파악은 `list_topics`, `get_topic_context`로 확인해 즉시 답변하세요.\n\
             3. 코드 작성·수정, 빌드, 리서치 등 실무 작업은 직접 처리하려 하지 말고 즉시 `delegate_to_topic` 도구로 하위 CLI 토픽 에이전트에게 위임한 뒤 지시 완료를 짧게 알리세요.\n\
             4. `[BACKGROUND_EVENT]`로 하위 토픽 작업 완료나 `AskQuestion`(확인 질문) 알림이 들어오면 즉시 사용자에게 음성으로 브리핑하고, 객관식 선택지가 있으면 번호와 함께 읽어준 뒤 사용자의 음성 답변을 `answer_topic_question`으로 전달하세요."
        )
    }
}

/// Builds the initial `BidiGenerateContent` `setup` JSON message.
pub fn build_live_setup_frame(
    lang: &str,
    active_topic: Option<&str>,
    resume_handle: Option<&str>,
) -> String {
    let resumption = match resume_handle.filter(|h| !h.is_empty()) {
        Some(h) => json!({ "handle": h }),
        None => json!({}),
    };
    json!({
        "setup": {
            "model": resolve_voice_model(),
            "generationConfig": {
                "responseModalities": ["AUDIO"],
                "speechConfig": {
                    "voiceConfig": {
                        "prebuiltVoiceConfig": { "voiceName": "Kore" }
                    }
                }
            },
            "inputAudioTranscription": {},
            "outputAudioTranscription": {},
            "contextWindowCompression": { "slidingWindow": {} },
            "sessionResumption": resumption,
            "systemInstruction": {
                "parts": [{ "text": build_secretary_prompt(lang, active_topic) }]
            },
            "tools": [{
                "functionDeclarations": voice_tool_declarations()
            }]
        }
    })
    .to_string()
}

/// Formats background `WebEvent` completions and `AskQuestion` prompts into live voice triggers.
pub fn format_background_web_event(ev: &WebEvent) -> Option<String> {
    match ev {
        WebEvent::TurnResult {
            session_key,
            text,
            is_error,
            ..
        } => {
            let status = if *is_error { "ERROR" } else { "COMPLETED" };
            let snippet: String = text.chars().take(500).collect();
            Some(format!(
                "[BACKGROUND_EVENT] Topic `{session_key}` {status}. Output summary:\n{snippet}\nPlease briefly inform the user by voice."
            ))
        }
        WebEvent::AskQuestion {
            session_key,
            questions,
            current_index,
            ..
        } => {
            let q = questions.get(*current_index)?;
            let opts = q
                .options
                .iter()
                .enumerate()
                .map(|(i, o)| format!("{}. {}", i + 1, o))
                .collect::<Vec<_>>()
                .join(" / ");
            Some(format!(
                "[BACKGROUND_EVENT] Topic `{session_key}` is waiting on AskQuestion ({}/{}): \"{}\" Options: [{}]. Please read this question and options clearly to the user so they can answer by voice.",
                current_index + 1,
                questions.len(),
                q.question,
                opts
            ))
        }
        _ => None,
    }
}

/// Parsed action emitted from a single upstream Gemini Live WebSocket JSON frame.
#[derive(Debug)]
pub enum UpstreamAction {
    /// Upstream `setupComplete` handshake succeeded.
    Ready,
    /// User barged in; browser must flush queued audio playback immediately.
    Interrupted,
    /// Base64-encoded 24 kHz 16-bit mono PCM audio chunk.
    AudioPcm24(String),
    /// Incremental user speech transcription.
    UserTranscript(String),
    /// Incremental assistant speech transcription.
    BotTranscript(String),
    /// Model turn completed.
    TurnComplete,
    /// Function call requested by Gemini Live `(id, name, args)`.
    ToolCalls(Vec<(String, String, Value)>),
    /// Updated `sessionResumption` token handle.
    ResumeHandle(String),
    /// Upstream `goAway` rotation warning; reconnect using stored `ResumeHandle`.
    GoAway,
    /// Upstream error message reported by Gemini Live.
    Error(String),
}

/// Maps a simple [`UpstreamAction`] directly to a browser WebSocket JSON payload when applicable.
pub fn action_to_client_frame(act: &UpstreamAction) -> Option<Value> {
    match act {
        UpstreamAction::Ready => {
            let m = resolve_voice_model().trim_start_matches("models/").to_string();
            Some(json!({ "type": "ready", "model": m }))
        }
        UpstreamAction::Interrupted => Some(json!({ "type": "interrupted" })),
        UpstreamAction::AudioPcm24(b) => Some(json!({ "type": "audio", "data": b })),
        UpstreamAction::UserTranscript(t) => Some(json!({ "type": "user_transcript", "text": t })),
        UpstreamAction::BotTranscript(t) => Some(json!({ "type": "bot_transcript", "text": t })),
        UpstreamAction::TurnComplete => Some(json!({ "type": "turn_complete" })),
        UpstreamAction::Error(e) => Some(json!({ "type": "error", "error": e })),
        _ => None,
    }
}

fn parse_server_content(sc: &Value, out: &mut Vec<UpstreamAction>) {
    if sc.get("interrupted").and_then(Value::as_bool) == Some(true) {
        out.push(UpstreamAction::Interrupted);
    }
    if let Some(parts) = sc.pointer("/modelTurn/parts").and_then(Value::as_array) {
        for p in parts {
            if let Some(b64) = p.pointer("/inlineData/data").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                out.push(UpstreamAction::AudioPcm24(b64.to_string()));
            }
        }
    }
    if let Some(ut) = sc.pointer("/inputTranscription/text").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        out.push(UpstreamAction::UserTranscript(ut.to_string()));
    }
    if let Some(bt) = sc.pointer("/outputTranscription/text").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        out.push(UpstreamAction::BotTranscript(bt.to_string()));
    }
    if sc.get("turnComplete").and_then(Value::as_bool) == Some(true) {
        out.push(UpstreamAction::TurnComplete);
    }
}

/// Parses an upstream Gemini Live JSON text frame into zero or more [`UpstreamAction`] items.
pub fn parse_gemini_server_frame(raw: &str) -> Vec<UpstreamAction> {
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if v.get("setupComplete").is_some() {
        out.push(UpstreamAction::Ready);
    }
    if let Some(sc) = v.get("serverContent") {
        parse_server_content(sc, &mut out);
    }
    if let Some(fcs) = v.pointer("/toolCall/functionCalls").and_then(Value::as_array) {
        let mut calls = Vec::new();
        for fc in fcs {
            let id = fc.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let name = fc.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            let args = fc.get("args").cloned().unwrap_or_else(|| json!({}));
            if !name.is_empty() {
                calls.push((id, name, args));
            }
        }
        if !calls.is_empty() {
            out.push(UpstreamAction::ToolCalls(calls));
        }
    }
    if let Some(sru) = v.get("sessionResumptionUpdate")
        && sru.get("resumable").and_then(Value::as_bool) == Some(true)
        && let Some(h) = sru.get("newHandle").and_then(Value::as_str).filter(|s| !s.is_empty())
    {
        out.push(UpstreamAction::ResumeHandle(h.to_string()));
    }
    if let Some(err_obj) = v.get("error") {
        let msg = err_obj.get("message").and_then(Value::as_str).unwrap_or("Gemini Live error");
        out.push(UpstreamAction::Error(msg.to_string()));
    }
    if v.get("goAway").is_some() {
        out.push(UpstreamAction::GoAway);
    }
    out
}
