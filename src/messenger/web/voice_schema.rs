//! # Voice Desk Secretary Tool Schema & Client Frame Translator (`voice_schema.rs`)
//!
//! ## Overview
//! Declares the 6 Gemini Live function-calling schemas (`list_topics`, `get_topic_context`,
//! `delegate_to_topic`, `answer_topic_question`, `stop_topic`, `switch_active_topic`) and
//! translates incoming browser WebSocket JSON frames into Gemini `realtimeInput` payloads.
//!
//! ## Search Tags
//! #voice-schema, #gemini-function-declarations, #realtime-input

use serde_json::{json, Value};

const DEFAULT_GEMINI_LIVE_URL: &str =
    "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";

/// Builds the upstream Gemini Live WebSocket URL with `?key=` attached.
pub fn build_upstream_url(api_key: &str) -> String {
    let base = std::env::var("TUNER_GEMINI_LIVE_WS_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_GEMINI_LIVE_URL.to_string());
    let sep = if base.contains('?') { '&' } else { '?' };
    format!("{base}{sep}key={api_key}")
}

/// Translates a browser `/dashboard/api/voice/ws` JSON frame into an upstream Gemini Live JSON string.
pub fn translate_browser_msg(raw: &str, current_topic: &mut Option<String>) -> Option<String> {
    let v: Value = serde_json::from_str(raw).ok()?;
    match v.get("type").and_then(Value::as_str)? {
        "audio" => {
            let b64 = v.get("data").and_then(Value::as_str)?.trim();
            (!b64.is_empty()).then(|| {
                json!({ "realtimeInput": { "audio": { "mimeType": "audio/pcm;rate=16000", "data": b64 } } }).to_string()
            })
        }
        "text" => {
            let txt = v.get("text").and_then(Value::as_str)?.trim();
            (!txt.is_empty()).then(|| json!({ "realtimeInput": { "text": txt } }).to_string())
        }
        "active_topic" => {
            *current_topic = v
                .get("session_key")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            None
        }
        _ => None,
    }
}

fn read_tool_decls() -> Vec<Value> {
    vec![
        json!({
            "name": "list_topics",
            "description": "Lists active Web and Telegram topics, their provider/model, and busy/asking state.",
            "parameters": { "type": "OBJECT", "properties": {} }
        }),
        json!({
            "name": "get_topic_context",
            "description": "Reads recent messages, live stream snippet, and open AskQuestion choices for a topic.",
            "parameters": {
                "type": "OBJECT",
                "properties": {
                    "session_key": { "type": "STRING", "description": "Optional topic key (defaults to active topic)." },
                    "max_messages": { "type": "INTEGER", "description": "Recent messages to read (default 6)." }
                }
            }
        }),
    ]
}

fn delegation_decls() -> Vec<Value> {
    vec![
        json!({
            "name": "delegate_to_topic",
            "description": "Delegates a coding, file, build, or research task asynchronously to a CLI topic.",
            "parameters": {
                "type": "OBJECT",
                "properties": {
                    "prompt": { "type": "STRING", "description": "Instruction for the CLI agent." },
                    "session_key": { "type": "STRING", "description": "Target topic key (omit for active/new)." },
                    "create_new_topic_title": { "type": "STRING", "description": "Title when creating a new topic." },
                    "transport": { "type": "STRING", "description": "Optional transport: 'web' or 'tg'." }
                },
                "required": ["prompt"]
            }
        }),
        json!({
            "name": "answer_topic_question",
            "description": "Submits the user's spoken answer to an open AskQuestion on a topic.",
            "parameters": {
                "type": "OBJECT",
                "properties": {
                    "session_key": { "type": "STRING", "description": "Target topic key." },
                    "option_index": { "type": "INTEGER", "description": "Zero-based option index (0, 1, ...)." },
                    "answer": { "type": "STRING", "description": "Custom free-text answer." },
                    "skip": { "type": "BOOLEAN", "description": "True to skip." }
                }
            }
        }),
    ]
}

fn control_decls() -> Vec<Value> {
    vec![
        json!({
            "name": "stop_topic",
            "description": "Soft-interrupts a running CLI turn on a topic.",
            "parameters": {
                "type": "OBJECT",
                "properties": { "session_key": { "type": "STRING", "description": "Target topic key." } }
            }
        }),
        json!({
            "name": "switch_active_topic",
            "description": "Switches the Web Dashboard screen to focus the specified topic.",
            "parameters": {
                "type": "OBJECT",
                "properties": { "session_key": { "type": "STRING", "description": "Topic key to display." } },
                "required": ["session_key"]
            }
        }),
    ]
}

/// Returns the Gemini Live `functionDeclarations` array for the Front-Desk Voice Secretary.
pub fn voice_tool_declarations() -> Value {
    let mut all = read_tool_decls();
    all.extend(delegation_decls());
    all.extend(control_decls());
    Value::Array(all)
}
