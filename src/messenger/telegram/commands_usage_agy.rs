//! # Antigravity Direct CloudCode Quota Client
//!
//! ## Overview
//! Queries Antigravity (`agy`) model quota directly via Google's CloudCode HTTP API
//! (`loadCodeAssist` + `retrieveUserQuotaSummary`) with automatic OAuth token refresh,
//! eliminating the 1-beat stale cache race condition in PTY `/usage` modal scraping.
//!
//! ## Search Tags
//! #antigravity-quota, #cloudcode-api, #oauth-refresh, #usage-command

use std::path::{Path, PathBuf};
use std::time::Duration;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use teloxide::prelude::*;
use crate::cli::antigravity::AntigravityCli;
use crate::session::{data::SessionData, manager::SessionManager};
use super::commands::send_reply;
use super::commands_usage::{
    query_quota_from_pty, render_quota_message, ModelQuotaGroup, QuotaLimit, QuotaReport,
};
use super::commands_usage_claude::format_reset_timestamp;

const AGY_USER_AGENT: &str = "antigravity/cli/1.5.4 (aidev_client; os_type=linux; arch=amd64; cl=1017748741; auth_method=consumer)";
const AGY_CLOUDCODE_BASE: &str = "https://daily-cloudcode-pa.googleapis.com";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

fn parse_agy_group_entries(g: &Value) -> Option<ModelQuotaGroup> {
    let (mut weekly, mut five_hour) = (None, None);
    let items_opt = g.get("buckets").or_else(|| g.get("entries")).and_then(Value::as_array);
    if let Some(items) = items_opt {
        for item in items {
            let bid = item.get("bucketId").or_else(|| item.get("quotaId")).and_then(Value::as_str).unwrap_or("");
            let win = item.get("window").and_then(Value::as_str).unwrap_or("");
            let label = item.get("displayName").or_else(|| item.get("label")).and_then(Value::as_str).unwrap_or("");
            let frac = item.get("remainingFraction").and_then(Value::as_f64);
            let detail = item.get("resetTime").and_then(Value::as_str).map(format_reset_timestamp).unwrap_or_default();
            let pct = (frac.unwrap_or(0.0) * 100.0).clamp(0.0, 100.0);
            let lim = QuotaLimit { pct, detail };
            if win == "weekly" || bid.ends_with("weekly") || label.contains("Weekly") {
                weekly = Some(lim);
            } else if win == "5h" || bid.ends_with("5h") || label.contains("5-Hour") || label.contains("Five Hour") {
                five_hour = Some(lim);
            }
        }
    }
    if weekly.as_ref().is_some_and(|w| w.pct == 0.0)
        && let Some(ref mut f) = five_hour
        && f.pct == 0.0
        && f.detail.is_empty()
    {
        f.detail = "Disabled: Weekly limit hit.".to_string();
    }
    (weekly.is_some() || five_hour.is_some()).then_some(ModelQuotaGroup { weekly, five_hour })
}

/// Parses the Google CloudCode `retrieveUserQuotaSummary` JSON response into a [`QuotaReport`].
pub fn parse_antigravity_quota_json(
    quota_json: &Value,
    email: Option<&str>,
    tier: Option<&str>,
) -> Result<QuotaReport, String> {
    let groups = quota_json
        .get("groups")
        .and_then(Value::as_array)
        .ok_or_else(|| "Missing 'groups' array in quota summary".to_string())?;

    let account = match (email.filter(|s| !s.is_empty()), tier.filter(|s| !s.is_empty())) {
        (Some(e), Some(t)) => Some(format!("{} ({})", e, t)),
        (Some(e), None) => Some(e.to_string()),
        (None, Some(t)) => Some(t.to_string()),
        (None, None) => None,
    };

    let (mut gemini, mut claude) = (None, None);
    for g in groups {
        let lower = g
            .get("displayName")
            .or_else(|| g.get("groupName"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        if let Some(group) = parse_agy_group_entries(g) {
            if lower.contains("gemini") {
                gemini = Some(group);
            } else if lower.contains("claude") || lower.contains("gpt") {
                claude = Some(group);
            }
        }
    }
    if gemini.is_none() && claude.is_none() {
        return Err("No recognized quota groups in response".to_string());
    }
    Ok(QuotaReport { account, gemini, claude, raw_fallback: String::new() })
}

async fn refresh_agy_oauth_token(
    client: &reqwest::Client,
    token_path: &Path,
    token_url: &str,
    refresh: &str,
    mut doc: Value,
) -> Result<String, String> {
    let client_id = std::env::var("AGY_OAUTH_CLIENT_ID").unwrap_or_else(|_| "antigravity-cli".to_string());
    let client_secret = std::env::var("AGY_OAUTH_CLIENT_SECRET").unwrap_or_default();
    let params = [
        ("client_id", client_id.as_str()),
        ("client_secret", client_secret.as_str()),
        ("refresh_token", refresh),
        ("grant_type", "refresh_token"),
    ];
    let resp: Value = client
        .post(token_url)
        .form(&params)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let new_access = resp
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| "Missing access_token".to_string())?
        .to_string();
    let expires_in = resp.get("expires_in").and_then(Value::as_i64).unwrap_or(3600);
    if let Some(t) = doc.get_mut("token").and_then(Value::as_object_mut) {
        t.insert("access_token".to_string(), Value::String(new_access.clone()));
        let exp = (Utc::now() + chrono::Duration::seconds(expires_in)).to_rfc3339();
        t.insert("expiry".to_string(), Value::String(exp));
        if let Ok(s) = serde_json::to_string(&doc) {
            let _ = tokio::fs::write(token_path, s).await;
        }
    }
    Ok(new_access)
}

async fn load_or_refresh_agy_token(
    client: &reqwest::Client,
    token_path: &Path,
    token_url: &str,
) -> Result<(String, Option<String>), String> {
    let raw = tokio::fs::read_to_string(token_path).await.map_err(|e| e.to_string())?;
    let doc: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let email = doc.get("id_token_email").and_then(Value::as_str).map(String::from);
    let tok_obj = doc.get("token").ok_or_else(|| "Missing 'token' field".to_string())?;
    let access = tok_obj.get("access_token").and_then(Value::as_str).unwrap_or("").to_string();
    let refresh = tok_obj.get("refresh_token").and_then(Value::as_str).unwrap_or("").to_string();
    let expiry_str = tok_obj.get("expiry").and_then(Value::as_str).unwrap_or("");

    let needs_refresh = DateTime::parse_from_rfc3339(expiry_str)
        .map(|dt| dt.with_timezone(&Utc).signed_duration_since(Utc::now()).num_seconds() < 60)
        .unwrap_or(access.is_empty());

    if !needs_refresh && !access.is_empty() {
        return Ok((access, email));
    }
    if refresh.is_empty() {
        return Err("OAuth access token expired and no refresh_token".to_string());
    }
    let new_access = refresh_agy_oauth_token(client, token_path, token_url, &refresh, doc).await?;
    Ok((new_access, email))
}

/// Queries Antigravity quota directly via the CloudCode HTTP API using the given endpoint URLs.
pub async fn query_antigravity_quota_http_with_urls(
    token_path: &Path,
    base_url: &str,
    token_url: &str,
) -> Result<QuotaReport, String> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(4)).build().map_err(|e| e.to_string())?;
    let (access_token, email) = load_or_refresh_agy_token(&client, token_path, token_url).await?;
    let base = base_url.trim_end_matches('/');

    let assist_val: Value = client
        .post(format!("{}/v1internal:loadCodeAssist", base))
        .bearer_auth(&access_token)
        .header(reqwest::header::USER_AGENT, AGY_USER_AGENT)
        .json(&json!({"metadata": {"ideType": "ANTIGRAVITY"}}))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .unwrap_or_else(|_| json!({}));

    let project = assist_val.get("cloudaicompanionProject").and_then(Value::as_str).unwrap_or("aicode-consumers");
    let tier = assist_val.get("paidTier").and_then(|t| t.get("name")).and_then(Value::as_str);

    let quota_val: Value = client
        .post(format!("{}/v1internal:retrieveUserQuotaSummary", base))
        .bearer_auth(&access_token)
        .header(reqwest::header::USER_AGENT, AGY_USER_AGENT)
        .json(&json!({"project": project}))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    parse_antigravity_quota_json(&quota_val, email.as_deref(), tier)
}

/// Queries Antigravity quota directly from Google's CloudCode endpoint using the default token file.
pub async fn query_antigravity_quota_http() -> Result<QuotaReport, String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let token_path = PathBuf::from(home).join(".gemini/antigravity-cli/antigravity-oauth-token");
    query_antigravity_quota_http_with_urls(&token_path, AGY_CLOUDCODE_BASE, GOOGLE_TOKEN_URL).await
}

/// Queries Antigravity quota via direct HTTP with non-intrusive fallback to an idle PTY session.
pub(crate) async fn query_antigravity_usage_html(
    bot: &Bot,
    msg: &Message,
    sessions: &SessionManager,
    cli: &AntigravityCli,
    sess: &SessionData,
) -> Option<String> {
    let http_err = match query_antigravity_quota_http().await {
        Ok(report) => return Some(render_quota_message(&report)),
        Err(e) => e,
    };
    let sid = sess.get_session_id("antigravity");
    let topic_id = crate::telegram::get_topic_id(msg);
    let lock = sessions.lock_pool.get((msg.chat.id.0, topic_id));
    if !sid.is_empty()
        && let Ok(_guard) = lock.try_lock()
        && !cli.sessions.is_running(&sid).await
    {
        let mut s_cli = cli.clone();
        s_cli.config.chat_id = msg.chat.id.0;
        s_cli.config.topic_id = sess.topic_id.or(topic_id);
        if !sess.model.is_empty() && sess.model != "antigravity-default" {
            s_cli.config.model = Some(sess.model.clone());
        }
        if sess.effort.is_some() {
            s_cli.config.effort = sess.effort.clone();
        }
        if let Ok(r) = query_quota_from_pty(&s_cli, &sid).await {
            return Some(render_quota_message(&r));
        }
    }
    let _ = send_reply(bot, msg, format!("⚠️ Could not retrieve quota: {}", http_err)).await;
    None
}
