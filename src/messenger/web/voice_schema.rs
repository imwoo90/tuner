//! # Voice Desk Secretary Tool Schema & Client Frame Translator (`voice_schema.rs`)
//!
//! ## Overview
//! Declares the 6 Gemini Live function-calling schemas (`list_topics`, `get_topic_context`,
//! `delegate_to_topic`, `answer_topic_question`, `stop_topic`, `switch_active_topic`) and
//! translates incoming browser WebSocket JSON frames into Gemini `realtimeInput` payloads.
//!
//! ## Search Tags
//! #voice-schema, #gemini-function-declarations, #realtime-input

use super::events::WebEvent;
use super::provider_status::read_dotenv_map;
use serde_json::{json, Value};
use std::path::Path;

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
