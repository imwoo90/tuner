//! # Claude Code Quota & Usage Client
//!
//! ## Overview
//! Implements native `/usage` telemetry for `claude` topics by querying Anthropic's
//! `/api/oauth/usage` endpoint (matching `claude`'s internal `fetchUtilization`) and
//! aggregating active session JSONL token/cache telemetry.
//!
//! ## Search Tags
//! #claude-oauth-usage, #claude-telemetry, #usage-command

use std::path::{Path, PathBuf};
use std::time::Duration;
use chrono::{DateTime, Utc};
use serde_json::Value;
use crate::session::data::SessionData;
use super::commands_usage::{render_group_markdown, ModelQuotaGroup, QuotaLimit};

/// Formats a remaining duration in seconds into a concise reset countdown string.
pub fn format_seconds_countdown(secs: i64) -> String {
    if secs <= 0 {
        return "Refreshes soon".to_string();
    }
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;
    if days > 0 {
        format!("Refreshes in {}d {}h", days, hours)
    } else if hours > 0 {
        format!("Refreshes in {}h {}m", hours, mins)
    } else {
        format!("Refreshes in {}m", mins.max(1))
    }
}

/// Formats an RFC3339 timestamp or Unix epoch seconds into a human-readable reset countdown.
pub fn format_reset_timestamp(raw: &str) -> String {
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        let secs = dt.with_timezone(&Utc).signed_duration_since(Utc::now()).num_seconds();
        return format_seconds_countdown(secs);
    }
    if let Ok(epoch) = raw.parse::<i64>() {
        return format_seconds_countdown(epoch - Utc::now().timestamp());
    }
    raw.to_string()
}

/// Parses Claude Code `/api/oauth/usage` JSON into a [`ModelQuotaGroup`].
pub fn parse_claude_oauth_usage_json(val: &Value) -> Option<ModelQuotaGroup> {
    let parse_win = |w: &Value| -> Option<QuotaLimit> {
        let used_raw = w.get("utilization").and_then(Value::as_f64)?;
        let used_pct = if used_raw > 0.0
            && used_raw <= 1.0
            && w.get("is_fraction").and_then(Value::as_bool).unwrap_or(false)
        {
            used_raw * 100.0
        } else {
            used_raw.clamp(0.0, 100.0)
        };
        let rem_pct = (100.0 - used_pct).clamp(0.0, 100.0);
        let reset_str = match w.get("resets_at") {
            Some(Value::String(s)) => format_reset_timestamp(s),
            Some(Value::Number(n)) => n
                .as_i64()
                .map(|ts| format_seconds_countdown(ts - Utc::now().timestamp()))
                .unwrap_or_default(),
            _ => String::new(),
        };
        let detail = if reset_str.is_empty() {
            format!("{:.0}% used", used_pct)
        } else {
            format!("{:.0}% used · {}", used_pct, reset_str)
        };
        Some(QuotaLimit { pct: rem_pct, detail })
    };
    let five_hour = val.get("five_hour").and_then(parse_win);
    let weekly = val.get("seven_day").and_then(parse_win);
    (weekly.is_some() || five_hour.is_some()).then_some(ModelQuotaGroup { weekly, five_hour })
}

fn read_claude_session_stats(session_id: &str) -> (u64, u64, u64, u64, usize) {
    if session_id.is_empty() {
        return (0, 0, 0, 0, 0);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let projects_dir = PathBuf::from(home).join(".claude/projects");
    let Ok(entries) = std::fs::read_dir(projects_dir) else {
        return (0, 0, 0, 0, 0);
    };
    let target = format!("{}.jsonl", session_id);
    for entry in entries.flatten() {
        let candidate = entry.path().join(&target);
        if let Ok(content) = std::fs::read_to_string(&candidate) {
            let (mut inp, mut outp, mut cread, mut cwrite, mut turns) = (0u64, 0u64, 0u64, 0u64, 0usize);
            for line in content.lines() {
                let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
                if v.get("type").and_then(Value::as_str) == Some("assistant")
                    && let Some(u) = v.get("message").and_then(|m| m.get("usage"))
                {
                    turns += 1;
                    inp += u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
                    outp += u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
                    cread += u.get("cache_read_input_tokens").and_then(Value::as_u64).unwrap_or(0);
                    cwrite += u.get("cache_creation_input_tokens").and_then(Value::as_u64).unwrap_or(0);
                }
            }
            return (inp, outp, cread, cwrite, turns);
        }
    }
    (0, 0, 0, 0, 0)
}

async fn fetch_claude_oauth_quota(creds_path: &Path, base_url: &str) -> (Option<String>, Option<ModelQuotaGroup>) {
    let Ok(raw) = tokio::fs::read_to_string(creds_path).await else { return (None, None) };
    let Ok(doc) = serde_json::from_str::<Value>(&raw) else { return (None, None) };
    let Some(oauth) = doc.get("claudeAiOauth") else { return (None, None) };
    let sub_type = oauth.get("subscriptionType").and_then(Value::as_str).map(String::from);
    let Some(tok) = oauth.get("accessToken").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
        return (sub_type, None);
    };
    let Ok(client) = reqwest::Client::builder().timeout(Duration::from_secs(4)).build() else {
        return (sub_type, None);
    };
    let url = format!("{}/api/oauth/usage", base_url.trim_end_matches('/'));
    let Ok(resp) = client.get(&url).bearer_auth(tok).send().await else {
        return (sub_type, None);
    };
    let Ok(val) = resp.json::<Value>().await else {
        return (sub_type, None);
    };
    (sub_type, parse_claude_oauth_usage_json(&val))
}

/// Queries Claude Code usage (OAuth `/api/oauth/usage` quota + session JSONL telemetry).
pub async fn query_claude_usage_html(
    sess: &SessionData,
    creds_override: Option<&Path>,
    base_url_override: Option<&str>,
) -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let default_creds = PathBuf::from(&home).join(".claude/.credentials.json");
    let creds_path = creds_override.unwrap_or(&default_creds);
    let base_url = base_url_override
        .map(String::from)
        .or_else(|| std::env::var("ANTHROPIC_BASE_URL").ok())
        .unwrap_or_else(|| "https://api.anthropic.com".to_string());

    let (sub_type, oauth_group) = fetch_claude_oauth_quota(creds_path, &base_url).await;
    let sid = sess.get_session_id("claude");
    let (inp, outp, cread, cwrite, turns) = read_claude_session_stats(&sid);
    let model = if sess.model.is_empty() { "claude-default" } else { &sess.model };
    let auth_label = sub_type
        .as_deref()
        .map(|s| format!("Claude OAuth ({})", s))
        .unwrap_or_else(|| "API Key / Local Bridge".to_string());

    let mut html = String::from("📊 <b>[tuner] Claude Code Quota & Usage</b>\n");
    html.push_str(&format!("🔑 <b>Auth</b>: <code>{}</code>\n", html_escape::encode_safe(&auth_label)));
    html.push_str(&format!("🤖 <b>Model</b>: <code>{}</code>\n", html_escape::encode_safe(model)));
    if let Some(ref grp) = oauth_group {
        html.push_str(&render_group_markdown("🧠", "CLAUDE SUBSCRIPTION QUOTA", grp));
    }
    html.push_str("\n📈 <b>SESSION TELEMETRY</b>\n");
    html.push_str(&format!("├ 🔹 Assistant Turns: <code>{}</code>\n", turns));
    html.push_str(&format!("├ 🔹 Input / Output: <code>{}</code> in / <code>{}</code> out\n", inp, outp));
    html.push_str(&format!("└ 🔹 Prompt Cache: <code>{}</code> read / <code>{}</code> write\n", cread, cwrite));
    html
}
