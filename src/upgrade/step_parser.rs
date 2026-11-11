//! # Antigravity Transcript Step Parser
//!
//! ## Overview
//! Parses individual JSONL transcript lines from Antigravity `transcript.jsonl` /
//! `transcript_full.jsonl` into compact `step_index`-keyed records (`CompactStep`),
//! normalizing embedded-quote scalar arguments, compound timer durations, and quiet heartbeats.
//!
//! ## Search Tags
//! #transcript-parser, #step-index, #schedule-timer, #tool-args

const HEARTBEAT_PROMPT_PREFIX: &str =
    "System self-check: are there any outstanding issues or background alerts to report?";

fn clean_json_scalar(val: Option<&serde_json::Value>) -> Option<String> {
    let v = val?;
    let raw = match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => return None,
    };
    let mut s = raw.trim();
    while s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s = s[1..s.len() - 1].trim();
    }
    (!s.is_empty()).then(|| s.to_string())
}

fn clean_arg_bool(args: &serde_json::Value, key: &str) -> bool {
    clean_json_scalar(args.get(key)).is_some_and(|s| s.eq_ignore_ascii_case("true"))
}

/// Sums all `<num><unit>` segments in `Task Description: Timer: 1h 30m 10s, Prompt: ...`.
pub fn parse_timer_desc_secs(content: &str) -> Option<u64> {
    let after = content.split("Task Description:").nth(1)?.trim_start();
    let timer = after.strip_prefix("Timer:")?;
    let header = timer
        .lines()
        .next()
        .unwrap_or(timer)
        .split(',')
        .next()
        .unwrap_or("");
    let mut rest = header.trim();
    let mut total = 0_u64;
    let mut found = false;
    while !rest.is_empty() {
        rest = rest.trim_start();
        let dlen = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if dlen == 0 {
            break;
        }
        let Ok(num) = rest[..dlen].parse::<u64>() else {
            break;
        };
        rest = rest[dlen..].trim_start();
        let (mult, ulen) = match rest.chars().next() {
            Some('s') => (1_u64, 1),
            Some('m') => (60, 1),
            Some('h') => (3600, 1),
            Some('d') => (86400, 1),
            _ => (1, 0),
        };
        rest = &rest[ulen..];
        total = total.saturating_add(num.saturating_mul(mult));
        found = true;
    }
    found.then_some(total)
}

#[derive(Clone)]
pub(crate) enum PendingCall {
    ManageTask {
        kill_tid: Option<String>,
    },
    Schedule {
        dur: Option<u64>,
        cond: String,
        is_daemon: bool,
    },
    RunCommand {
        is_daemon: bool,
    },
    Other,
}

pub(crate) struct CompactStep {
    pub(crate) stype: String,
    pub(crate) status: String,
    pub(crate) step_ts: u64,
    pub(crate) has_tools: bool,
    pub(crate) is_hb_prompt: bool,
    pub(crate) is_hb_ack: bool,
    pub(crate) calls: Vec<PendingCall>,
    pub(crate) content: Option<String>,
}

fn is_relevant_content(stype: &str, status: &str, c: &str) -> bool {
    status == "RUNNING"
        || stype == "SYSTEM_MESSAGE"
        || (stype == "GENERIC"
            && status == "DONE"
            && (c.contains("No background tasks are currently running.")
                || c.contains("background task(s) currently running:")
                || c.contains(" cancelled.")
                || (c.contains("Task:") && c.contains("Status:"))))
}

fn extract_calls(val: &serde_json::Value) -> Vec<PendingCall> {
    let Some(arr) = val.get("tool_calls").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for tc in arr {
        let name = tc.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let args = tc.get("args").or_else(|| tc.get("arguments"));
        let item = match name {
            "manage_task" => {
                let kill_tid = args.and_then(|a| {
                    (clean_json_scalar(a.get("Action")).as_deref() == Some("kill"))
                        .then(|| clean_json_scalar(a.get("TaskId")))
                        .flatten()
                });
                PendingCall::ManageTask { kill_tid }
            }
            "schedule" => {
                let (dur, cond, is_daemon) = if let Some(a) = args {
                    (
                        clean_json_scalar(a.get("DurationSeconds"))
                            .and_then(|s| s.parse::<u64>().ok()),
                        clean_json_scalar(a.get("TimerCondition"))
                            .unwrap_or_else(|| "never".into()),
                        clean_arg_bool(a, "IsDaemon"),
                    )
                } else {
                    (None, "never".into(), false)
                };
                PendingCall::Schedule {
                    dur,
                    cond,
                    is_daemon,
                }
            }
            "run_command" => PendingCall::RunCommand {
                is_daemon: args.is_some_and(|a| clean_arg_bool(a, "IsDaemon")),
            },
            _ => PendingCall::Other,
        };
        out.push(item);
    }
    out
}

pub(crate) fn parse_compact_step_with_ack(
    line: &str,
    seq: u64,
    fallback_epoch: u64,
    ack_token: &str,
) -> Option<((u8, u64), CompactStep)> {
    let val = serde_json::from_str::<serde_json::Value>(line).ok()?;
    let key = match val.get("step_index").and_then(|v| v.as_u64()) {
        Some(idx) => (0_u8, idx),
        None => (1_u8, seq),
    };
    let stype = val.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let status = val.get("status").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let step_ts = val
        .get("created_at")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.timestamp().max(0) as u64)
        .unwrap_or(fallback_epoch);
    let calls = if stype == "PLANNER_RESPONSE" { extract_calls(&val) } else { Vec::new() };
    let has_tools = val
        .get("tool_calls")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());
    let raw_content = val.get("content").and_then(|v| v.as_str());
    let is_hb_prompt = stype == "USER_INPUT"
        && raw_content.is_some_and(|c| c.contains(HEARTBEAT_PROMPT_PREFIX));
    let clean_ack = ack_token.trim();
    let is_hb_ack = stype == "PLANNER_RESPONSE"
        && !has_tools
        && raw_content.is_some_and(|c| {
            let t = c.trim();
            t == "HEARTBEAT_OK" || (!clean_ack.is_empty() && t == clean_ack)
        });
    let content = raw_content
        .filter(|c| is_relevant_content(&stype, &status, c))
        .map(|c| c.to_string());
    Some((
        key,
        CompactStep { stype, status, step_ts, has_tools, is_hb_prompt, is_hb_ack, calls, content },
    ))
}
