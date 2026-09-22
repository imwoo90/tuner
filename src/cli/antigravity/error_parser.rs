//! # Smart CLI Error Parser
//!
//! This module analyzes agy CLI process exit status and stderr streams
//! to offer clear, actionable restoration guidelines to the user.
//! In agy 1.2.6+, headless runs exit with code 3 on agent/model API failures
//! and emit structured JSON AGY_ERROR on stderr.
//!
//! ## Search Tags
//! #error-parser

use serde::Deserialize;

#[derive(Deserialize, Debug, Clone, Default, PartialEq)]
pub struct AgyErrorPayload {
    pub status: Option<String>,
    pub code: Option<serde_json::Value>,
    pub message: Option<String>,
    pub retryable: Option<bool>,
    pub error_id: Option<String>,
    pub error: Option<serde_json::Value>,
}

impl AgyErrorPayload {
    pub fn code_as_i64(&self) -> Option<i64> {
        let v = self.code.as_ref().or_else(|| self.error.as_ref()?.get("code"))?;
        v.as_i64().or_else(|| v.as_str()?.parse().ok())
    }

    pub fn code_str(&self) -> Option<&str> {
        self.code.as_ref().or_else(|| self.error.as_ref()?.get("code"))?.as_str()
    }

    pub fn effective_status(&self) -> Option<&str> {
        self.status.as_deref().or_else(|| self.error.as_ref()?.get("status")?.as_str())
    }

    pub fn effective_message(&self) -> Option<&str> {
        self.message.as_deref().or_else(|| self.error.as_ref()?.get("message")?.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.status.is_none() && self.code.is_none() && self.message.is_none()
            && self.error_id.is_none() && self.error.is_none()
    }
}

const ERR_RATE_LIMIT: &str = "❌ [tuner] Model API rate limit or quota exceeded.";
const ERR_CONTEXT_LIMIT: &str = "❌ [tuner] Context window or token limit exceeded.";
const ERR_UNAVAILABLE: &str = "❌ [tuner] Model service is temporarily overloaded or unavailable.";
const ERR_NOT_FOUND: &str = "❌ [tuner] Model not found or unsupported.";
const ERR_AUTH: &str = "❌ [tuner] Model API authentication or permission error.";

const SUGG_RETRY_LATER: &str = "💡 Suggestion: The provider rate limit was reached. Please wait a short while before retrying. If your monthly quota is exhausted, check your API billing/quota settings.";
const SUGG_QUOTA: &str = "💡 Suggestion: Your model provider quota has been exhausted. Check your API billing settings or switch to a different model with /model.";
const SUGG_CONTEXT: &str = "💡 Suggestion: The prompt or conversation history exceeds the model's context window. Start a fresh session with /new to reset context.";
const SUGG_OVERLOADED: &str = "💡 Suggestion: The upstream provider is experiencing high traffic or temporary downtime. Try again in a few moments, or switch models using /model.";
const SUGG_MODEL_404: &str = "💡 Suggestion: The selected model does not exist or is not available on this endpoint. Use /model to switch to a valid model.";
const SUGG_AUTH: &str = "💡 Suggestion: Verify your API key in ~/.tuner/.env or check permissions for your account.";

const MSG_CODE_3: &str = "❌ [tuner] Antigravity model or agent failure (exit code 3).\n💡 Suggestion: The model provider or agent failed to complete the turn. Try again in a moment, or switch models with /model.";
const MSG_MISSING_KEY: &str = "❌ [tuner] API key is missing or not configured.\n💡 Suggestion: Write your API key into ~/.tuner/.env (e.g. ANTHROPIC_API_KEY=sk-...) to make it globally available.";
const MSG_TIMEOUT: &str = "❌ [tuner] Antigravity execution timed out.\n💡 Suggestion: The turn took longer than the configured timeout limit. You can retry with a smaller prompt, or extend the execution timeout.";
const MSG_PERM_DENIED: &str = "❌ [tuner] Permission denied when executing agy.\n💡 Suggestion: Check execute permissions (chmod +x) for the agy binary or workspace path.";
const MSG_NOT_FOUND: &str = "❌ [tuner] agy CLI is not installed or not in PATH.\n💡 Suggestion: Install agy CLI and verify it is accessible from the system PATH.";

fn has_any(text: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|&p| text.contains(p))
}

fn try_parse_payload(s: &str) -> Option<AgyErrorPayload> {
    if let Ok(p) = serde_json::from_str::<AgyErrorPayload>(s) {
        if !p.is_empty() { return Some(p); }
    }
    let mut de = serde_json::Deserializer::from_str(s).into_iter::<AgyErrorPayload>();
    if let Some(Ok(p)) = de.next() {
        if !p.is_empty() { return Some(p); }
    }
    None
}

pub fn extract_agy_error(stderr: &str) -> Option<AgyErrorPayload> {
    const TAG: &str = "AGY_ERROR:";
    for line in stderr.lines() {
        if let Some(idx) = line.find(TAG) {
            if let Some(p) = try_parse_payload(line[idx + TAG.len()..].trim()) { return Some(p); }
        }
        let t = line.trim();
        if t.starts_with('{') && (t.contains("\"status\"") || t.contains("\"message\"") || t.contains("\"code\"")) {
            if let Some(p) = try_parse_payload(t) { return Some(p); }
        }
    }
    let idx = stderr.find(TAG)?;
    try_parse_payload(stderr[idx + TAG.len()..].trim_start())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgyErrorKind {
    RateLimit,
    ContextLength,
    Unavailable,
    NotFound,
    Auth,
    General,
}

fn classify_agy_error(payload: &AgyErrorPayload) -> AgyErrorKind {
    let (c_num, c_str) = (payload.code_as_i64(), payload.code_str().unwrap_or("").to_uppercase());
    let status = payload.effective_status().unwrap_or("").to_uppercase();
    let msg = payload.effective_message().unwrap_or("").trim().to_lowercase();
    let (s, c) = (status.as_str(), c_str.as_str());

    if c_num == Some(429) || s == "RESOURCE_EXHAUSTED" || c == "RESOURCE_EXHAUSTED"
        || has_any(&msg, &["quota", "rate limit", "resource exhausted", "too many requests"]) {
        return AgyErrorKind::RateLimit;
    }
    if has_any(&msg, &["context length", "context window", "token limit", "prompt length", "prompt too long", "prompt is too long", "too many tokens"])
        || (s == "INVALID_ARGUMENT" && has_any(&msg, &["token", "context", "length"])) {
        return AgyErrorKind::ContextLength;
    }
    if matches!(c_num, Some(500..=504 | 529)) || matches!(s, "UNAVAILABLE" | "INTERNAL" | "DEADLINE_EXCEEDED") || matches!(c, "UNAVAILABLE" | "INTERNAL")
        || has_any(&msg, &["overloaded", "unavailable", "server error", "high demand", "capacity"]) {
        return AgyErrorKind::Unavailable;
    }
    if c_num == Some(404) || s == "NOT_FOUND" || c == "NOT_FOUND"
        || has_any(&msg, &["model not found", "model does not exist", "unknown model"]) {
        return AgyErrorKind::NotFound;
    }
    if matches!(c_num, Some(401 | 403)) || matches!(s, "PERMISSION_DENIED" | "UNAUTHENTICATED") || matches!(c, "PERMISSION_DENIED" | "UNAUTHENTICATED")
        || has_any(&msg, &["unauthorized", "invalid api key", "permission denied"]) {
        return AgyErrorKind::Auth;
    }
    AgyErrorKind::General
}

fn get_error_guidance(kind: AgyErrorKind, payload: &AgyErrorPayload) -> (&'static str, &'static str) {
    let msg = payload.effective_message().unwrap_or("").to_lowercase();
    match kind {
        AgyErrorKind::RateLimit => {
            let s = if payload.retryable == Some(true) {
                SUGG_RETRY_LATER
            } else if payload.retryable == Some(false) || msg.contains("quota") {
                SUGG_QUOTA
            } else {
                SUGG_RETRY_LATER
            };
            (ERR_RATE_LIMIT, s)
        }
        AgyErrorKind::ContextLength => (ERR_CONTEXT_LIMIT, SUGG_CONTEXT),
        AgyErrorKind::Unavailable => (ERR_UNAVAILABLE, SUGG_OVERLOADED),
        AgyErrorKind::NotFound => (ERR_NOT_FOUND, SUGG_MODEL_404),
        AgyErrorKind::Auth => (ERR_AUTH, SUGG_AUTH),
        AgyErrorKind::General => {
            let s = if payload.retryable == Some(true) {
                "💡 Suggestion: The request failed with a retryable error. You can retry your message in a moment."
            } else {
                "💡 Suggestion: The model provider returned an error. Check the details below or try again with /new or /model."
            };
            ("❌ [tuner] Antigravity model API error.", s)
        }
    }
}

pub fn format_agy_error(payload: &AgyErrorPayload) -> String {
    let kind = classify_agy_error(payload);
    let (header, suggestion) = get_error_guidance(kind, payload);
    let title = if kind == AgyErrorKind::General {
        if let Some(s) = payload.effective_status().filter(|s| !s.is_empty()) {
            format!("❌ [tuner] Antigravity model API error (status: {}).", s.to_uppercase())
        } else if let Some(c) = payload.code_as_i64() {
            format!("❌ [tuner] Antigravity model API error (code: {}).", c)
        } else {
            header.to_string()
        }
    } else {
        header.to_string()
    };

    let mut out = format!("{}\n{}", title, suggestion);
    if let Some(msg) = payload.effective_message() {
        let trimmed = msg.trim();
        if !trimmed.is_empty() {
            out.push_str(&format!("\n\nDetails: {}", trimmed));
        }
    }
    if let Some(ref eid) = payload.error_id {
        out.push_str(&format!("\nError ID: {}", eid));
    }
    out
}

pub fn parse_cli_error(stderr: &str, returncode: i32) -> String {
    if let Some(payload) = extract_agy_error(stderr) {
        return format_agy_error(&payload);
    }
    let lower = stderr.to_lowercase();
    if returncode == 3 || lower.contains("exit code 3") || lower.contains("exit status: 3") || lower.contains("unix_wait_status(768)") {
        let trimmed = stderr.trim();
        return if trimmed.is_empty() {
            MSG_CODE_3.to_string()
        } else {
            format!("{}\n\nDetails: {}", MSG_CODE_3, trimmed)
        };
    }
    if has_any(&lower, &["api key not found", "api_key not found", "api key is missing"]) {
        return MSG_MISSING_KEY.to_string();
    }
    if lower.starts_with("command timed out") || lower.contains("command timed out after") {
        return MSG_TIMEOUT.to_string();
    }
    if returncode == 126
        || (lower.contains("permission denied") && has_any(&lower, &["agy", "/bin", "exec"]))
    {
        return MSG_PERM_DENIED.to_string();
    }
    if returncode == 127 || has_any(&lower, &["command not found", "agy: not found"]) {
        return MSG_NOT_FOUND.to_string();
    }
    stderr.to_string()
}
