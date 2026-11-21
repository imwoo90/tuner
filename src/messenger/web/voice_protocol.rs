//! # Gemini Live (`gemini-3.8-live`) Wire Protocol Codec (`voice_protocol.rs`)
//!
//! ## Overview
//! Encodes `BidiGenerateContent` setup, realtime audio/text inputs, tool responses, and
//! background event briefings, and decodes incoming Gemini Live audio/transcript/tool frames.
//!
//! ## Search Tags
//! #gemini-live-protocol, #bidi-audio, #session-resumption, #voice-codec

use super::voice_schema::voice_tool_declarations;
pub use super::voice_schema::{resolve_gemini_api_key, resolve_voice_model};
use serde_json::{json, Value};

fn en_secretary_prompt(topic_hint: &str) -> String {
    format!(
        "You are Woo-Tuner ([우튜너]), the user's real-time Front-Desk Voice Secretary in front of the Tuner multi-agent workspace. {topic_hint}\n\
         Rules:\n\
         1. Speak naturally and concisely (1-3 short sentences). Never read Markdown symbols like `**` or tables aloud.\n\
         2. Answer quick checks via `list_topics`, `get_topic_context`, or `list_available_models`.\n\
         3. Delegate coding, builds, or deep research via `delegate_to_topic` (pass `provider`/`model`/`effort` whenever requested, e.g. `gemini-4-hydrogen`, `claude`, `codex`; for parallel work, call `delegate_to_topic` multiple times in one turn with distinct `create_new_topic_title`s).\n\
         4. On `[BATCH_COMPLETION_BRIEFING]` or `[BACKGROUND_EVENT]`, synthesize all completed topics together toward the user's goal instead of reading each room verbatim; read `AskQuestion` options clearly and submit answers via `answer_topic_question`.\n\
         5. Ignore unintelligible ambient noise or distant chatter not addressed to you."
    )
}

fn ko_secretary_prompt(topic_hint: &str) -> String {
    format!(
        "당신은 사용자의 멀티 에이전트 워크스페이스 앞단을 지키는 총괄 음성 비서 '우튜너'입니다. {topic_hint}\n\
         수칙:\n\
         1. 실제 통화처럼 자연스럽고 간결하게(1~3문장) 한국어로 말하세요. 마크다운 기호(**·백틱·표)는 절대 소리 내어 읽지 마세요.\n\
         2. 현황·모델 확인은 `list_topics`, `get_topic_context`, `list_available_models`로 즉시 확인하세요.\n\
         3. 실무·조사 작업은 `delegate_to_topic`으로 위임하세요. 사용자가 모델이나 엔진(예: `gemini-4-hydrogen`, `neon`, `argon`, `claude`, `codex`)을 지정하면 반드시 `provider`/`model`/`effort` 인자에 담아 실행하고(기존 방 변경은 `configure_topic`), 병렬 지시는 한 턴에 `create_new_topic_title`을 각각 다르게 주어 동시에 띄우세요.\n\
         4. `[BATCH_COMPLETION_BRIEFING]`이나 `[BACKGROUND_EVENT]` 수신 시 각 토픽 본문을 하나씩 줄줄 읽지 말고, 사용자의 처음 지시 의도에 맞춰 핵심 결론과 비교 요점을 하나로 엮어 **3~5문장으로 종합 보고**하세요 (`AskQuestion`은 선택지를 번호와 함께 읽어주고 `answer_topic_question`으로 응답 전달).\n\
         5. 주변 소음이나 멀리서 들리는 제3자의 외마디 말소리는 되묻지 말고 조용히 무시하세요."
    )
}

fn auto_secretary_prompt(topic_hint: &str) -> String {
    format!(
        "You are Woo-Tuner ([우튜너]), the user's real-time Front-Desk Voice Secretary in front of the Tuner multi-agent workspace. {topic_hint}\n\
         Rules:\n\
         1. Reply naturally in whichever language the user just spoke (1-3 short sentences). Never read Markdown symbols aloud.\n\
         2. Pass explicit `provider`/`model`/`effort` (e.g. `gemini-4-hydrogen`) on `delegate_to_topic` or `configure_topic`; invoke `delegate_to_topic` multiple times with distinct `create_new_topic_title`s for parallel tasks.\n\
         3. On `[BATCH_COMPLETION_BRIEFING]` or `[BACKGROUND_EVENT]`, deliver a unified executive synthesis tailored to the user's goal rather than reading each topic separately; handle `AskQuestion` via `answer_topic_question`.\n\
         4. Ignore ambient noise or distant chatter not addressed to you."
    )
}

fn build_secretary_prompt(lang: &str, active_topic: Option<&str>) -> String {
    let topic_hint = active_topic
        .filter(|s| !s.is_empty())
        .map(|s| format!("Currently open topic on user's screen: `{s}`."))
        .unwrap_or_else(|| "User is currently on a blank Draft screen (no topic selected yet).".to_string());
    if lang.eq_ignore_ascii_case("auto") {
        auto_secretary_prompt(&topic_hint)
    } else if lang.starts_with("en") {
        en_secretary_prompt(&topic_hint)
    } else {
        ko_secretary_prompt(&topic_hint)
    }
}

fn realtime_vad_config() -> Value {
    json!({
        "automaticActivityDetection": {
            "startOfSpeechSensitivity": "START_SENSITIVITY_LOW",
            "endOfSpeechSensitivity": "END_SENSITIVITY_LOW",
            "prefixPaddingMs": 180,
            "silenceDurationMs": 600
        }
    })
}

fn build_speech_config(lang: &str) -> Value {
    if lang.eq_ignore_ascii_case("auto") {
        json!({ "voiceConfig": { "prebuiltVoiceConfig": { "voiceName": "Kore" } } })
    } else {
        let code = if lang.starts_with("en") { "en-US" } else { "ko-KR" };
        json!({
            "languageCode": code,
            "voiceConfig": { "prebuiltVoiceConfig": { "voiceName": "Kore" } }
        })
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
                "speechConfig": build_speech_config(lang)
            },
            "realtimeInputConfig": realtime_vad_config(),
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
