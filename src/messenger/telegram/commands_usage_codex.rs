//! # OpenAI Codex Quota & Usage Client
//!
//! ## Overview
//! Implements native `/usage` telemetry for `codex` topics via `codex app-server --stdio`
//! JSON-RPC 2.0 (`account/read` + `account/rateLimits/read`) and active session `.jsonl`
//! `token_count` event extraction.
//!
//! ## Search Tags
//! #codex-rate-limits, #codex-telemetry, #usage-command

use std::path::{Path, PathBuf};
use std::time::Duration;
use chrono::Utc;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use crate::session::data::SessionData;
use super::commands_usage::{format_progress_bar, render_group_markdown, ModelQuotaGroup, QuotaLimit};
use super::commands_usage_claude::format_seconds_countdown;

/// Parses OpenAI Codex `GetAccountRateLimitsResponse` or `token_count.rate_limits` JSON.
pub fn parse_codex_rate_limits_json(val: &Value) -> (Option<String>, Option<ModelQuotaGroup>) {
    let rl = val.get("rateLimits").or_else(|| val.get("rate_limits")).unwrap_or(val);
    let plan = rl
        .get("planType")
        .or_else(|| rl.get("plan_type"))
        .and_then(Value::as_str)
        .map(String::from);

    let parse_win = |w: &Value| -> Option<QuotaLimit> {
        let used = w.get("usedPercent").or_else(|| w.get("used_percent")).and_then(Value::as_i64)?;
        let rem = (100.0 - (used as f64)).clamp(0.0, 100.0);
        let reset = w
            .get("resetsAt")
            .or_else(|| w.get("resets_at"))
            .and_then(Value::as_i64)
            .map(|ts| format_seconds_countdown(ts - Utc::now().timestamp()))
            .unwrap_or_default();
        let detail = if reset.is_empty() {
            format!("{}% used", used)
        } else {
            format!("{}% used · {}", used, reset)
        };
        Some(QuotaLimit { pct: rem, detail })
    };
    let five_hour = rl.get("primary").and_then(parse_win);
    let weekly = rl.get("secondary").and_then(parse_win);
    let group = (weekly.is_some() || five_hour.is_some()).then_some(ModelQuotaGroup { weekly, five_hour });
    (plan, group)
}

fn parse_codex_account_obj(acc: &Value) -> String {
    let kind = acc.get("type").and_then(Value::as_str).unwrap_or("account");
    let email = acc.get("email").and_then(Value::as_str).unwrap_or("");
    let plan = acc.get("planType").and_then(Value::as_str).unwrap_or("");
    match (email.is_empty(), plan.is_empty()) {
        (false, false) => format!("{} ({})", email, plan),
        (false, true) => email.to_string(),
        _ => kind.to_string(),
    }
}

async fn read_codex_rpc_responses(stdout: tokio::process::ChildStdout) -> (Option<String>, Option<ModelQuotaGroup>) {
    let mut reader = BufReader::new(stdout).lines();
    let (mut account_label, mut rate_group) = (None, None);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(2500);
    while tokio::time::Instant::now() < deadline {
        let Ok(Ok(Some(line))) = tokio::time::timeout(Duration::from_millis(800), reader.next_line()).await else {
            break;
        };
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        match v.get("id").and_then(Value::as_i64) {
            Some(2) => {
                if let Some(acc) = v.get("result").and_then(|r| r.get("account")).filter(|a| !a.is_null()) {
                    account_label = Some(parse_codex_account_obj(acc));
                }
            }
            Some(3) => {
                if let Some(res) = v.get("result") {
                    let (plan, grp) = parse_codex_rate_limits_json(res);
                    if account_label.is_none() { account_label = plan; }
                    rate_group = grp;
                }
                break;
            }
            _ => {}
        }
    }
    (account_label, rate_group)
}

async fn query_codex_app_server_rate_limits() -> (Option<String>, Option<ModelQuotaGroup>) {
    let Ok(mut child) = tokio::process::Command::new("codex")
        .args(["app-server", "--stdio"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
    else {
        return (None, None);
    };
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return (None, None);
    };
    let reqs = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"tuner","version":env!("CARGO_PKG_VERSION")}}}),
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"account/read","params":{"refreshToken":false}}),
        json!({"jsonrpc":"2.0","id":3,"method":"account/rateLimits/read","params":null}),
    ];
    for r in &reqs {
        if stdin.write_all(format!("{}\n", r).as_bytes()).await.is_err() {
            return (None, None);
        }
    }
    let _ = stdin.flush().await;
    read_codex_rpc_responses(stdout).await
}

fn find_codex_session_jsonl(dir: &Path, session_id: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(found) = find_codex_session_jsonl(&p, session_id) {
                return Some(found);
            }
        } else if p
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.contains(session_id) && n.ends_with(".jsonl"))
        {
            return Some(p);
        }
    }
    None
}

type CodexSessionStats = (u64, u64, u64, u64, u64, u64, Option<String>, Option<ModelQuotaGroup>);

async fn read_codex_session_telemetry(session_id: &str) -> CodexSessionStats {
    let mut out = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64, None, None);
    if session_id.is_empty() {
        return out;
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let sessions_dir = PathBuf::from(home).join(".codex/sessions");
    let Some(path) = find_codex_session_jsonl(&sessions_dir, session_id) else {
        return out;
    };
    let Ok(content) = tokio::fs::read_to_string(path).await else {
        return out;
    };
    for line in content.lines().rev() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let Some(payload) = v.get("payload") else { continue };
        if payload.get("type").and_then(Value::as_str) == Some("token_count") {
            if let Some(info) = payload.get("info") {
                out.5 = info.get("model_context_window").and_then(Value::as_u64).unwrap_or(0);
                if let Some(tu) = info.get("total_token_usage") {
                    out.0 = tu.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
                    out.1 = tu.get("cached_input_tokens").and_then(Value::as_u64).unwrap_or(0);
                    out.2 = tu.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
                    out.3 = tu.get("reasoning_output_tokens").and_then(Value::as_u64).unwrap_or(0);
                    out.4 = tu.get("total_tokens").and_then(Value::as_u64).unwrap_or(0);
                }
            }
            let (plan, grp) = parse_codex_rate_limits_json(payload);
            out.6 = plan;
            out.7 = grp;
            break;
        }
    }
    out
}

/// Queries OpenAI Codex usage (`account/rateLimits/read` JSON-RPC + session `token_count` telemetry).
pub async fn query_codex_usage_html(sess: &SessionData) -> String {
    let (mut acc_label, mut rate_group) = query_codex_app_server_rate_limits().await;
    let sid = sess.get_session_id("codex");
    let (inp, cached, outp, reas, total, ctx_win, fb_plan, fb_grp) =
        read_codex_session_telemetry(&sid).await;
    if acc_label.is_none() {
        acc_label = fb_plan;
    }
    if rate_group.is_none() {
        rate_group = fb_grp;
    }

    let model = if sess.model.is_empty() { "codex-default" } else { &sess.model };
    let auth_str = acc_label.as_deref().unwrap_or("API Key / Custom Provider");
    let mut html = String::from("📊 <b>[tuner] OpenAI Codex Quota & Usage</b>\n");
    html.push_str(&format!("🔑 <b>Auth</b>: <code>{}</code>\n", html_escape::encode_safe(auth_str)));
    html.push_str(&format!("🤖 <b>Model</b>: <code>{}</code>\n", html_escape::encode_safe(model)));
    if let Some(ref grp) = rate_group {
        html.push_str(&render_group_markdown("⚡", "CODEX RATE LIMITS", grp));
    }
    html.push_str("\n📈 <b>SESSION TELEMETRY</b>\n");
    html.push_str(&format!("├ 🔹 Input (Cached): <code>{}</code> (<code>{}</code> cached)\n", inp, cached));
    html.push_str(&format!("├ 🔹 Output (Reasoning): <code>{}</code> (<code>{}</code> reasoning)\n", outp, reas));
    if ctx_win > 0 {
        let rem_pct = (100.0 - ((total as f64 / ctx_win as f64) * 100.0)).clamp(0.0, 100.0);
        html.push_str(&format!(
            "└ 🔹 Context Window: <code>{}</code> {:.1}% free ({}/{})\n",
            format_progress_bar(rem_pct),
            rem_pct,
            total,
            ctx_win
        ));
    } else {
        html.push_str(&format!("└ 🔹 Total Tokens: <code>{}</code>\n", total));
    }
    html
}
