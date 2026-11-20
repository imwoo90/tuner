//! # Antigravity Turn Slice Scanner & Async Input Contention Guard (`turn_settle.rs`)
//!
//! ## Overview
//! Prevents foreground user input from colliding with background/post-turn `SYSTEM_MESSAGE` steps
//! on warm `agy` PTYs by settling active background turns prior to stdin injection, anchoring
//! turn completion to the trailing JSONL state, and preserving both background and user answers.
//!
//! ## Search Tags
//! #turn-settle, #async-race, #stdin-contention, #transcript-slice

use super::completion::{is_completion_entry, is_thinking_only_entry};
use super::events::is_placeholder_content;
use super::session::SessionManager;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Trailing completion state across all complete JSONL lines in `[turn_start_pos .. EOF]`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct TurnSliceScan {
    /// True when the latest turn in the slice is an unanswered standalone `SYSTEM_MESSAGE` turn.
    pub in_system_turn: bool,
    /// True when a `USER_INPUT` turn has started and has not yet reached a terminal `PLANNER_RESPONSE`.
    pub awaiting_user_response: bool,
    /// True when at least one `USER_INPUT` entry appeared in the slice.
    pub saw_user_input: bool,
    /// True when the most recent turn step in the slice is a terminal `PLANNER_RESPONSE`.
    pub latest_completed: bool,
    /// True when the terminal `PLANNER_RESPONSE` invoked `ask_question` or `ask_permission`.
    pub has_interactive_ask: bool,
    /// Byte offset immediately following the last complete `\n`-terminated line.
    pub next_pos: u64,
}

impl TurnSliceScan {
    /// Returns `true` when the foreground turn has reached a genuine terminal boundary.
    pub(crate) fn is_turn_complete(&self) -> bool {
        self.has_interactive_ask || (!self.in_system_turn && self.latest_completed)
    }

    /// Returns `true` when a background `SYSTEM_MESSAGE` turn completed before `USER_INPUT` arrived.
    pub(crate) fn needs_prompt_reinject(&self) -> bool {
        self.in_system_turn && self.latest_completed && !self.saw_user_input && !self.has_interactive_ask
    }
}

fn entry_has_interactive_tool(val: &serde_json::Value) -> bool {
    val.get("tool_calls")
        .and_then(|v| v.as_array())
        .is_some_and(|arr| {
            arr.iter().any(|tc| {
                matches!(tc.get("name").and_then(|n| n.as_str()), Some("ask_question" | "ask_permission"))
            })
        })
}

fn apply_scan_line(scan: &mut TurnSliceScan, line: &str) {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else { return };
    let etype = val.get("type").and_then(|s| s.as_str());
    let source = val.get("source").and_then(|s| s.as_str());
    if etype == Some("SYSTEM_MESSAGE") {
        if !scan.awaiting_user_response {
            scan.in_system_turn = true;
        }
        scan.latest_completed = false;
        scan.has_interactive_ask = false;
    } else if etype == Some("USER_INPUT") {
        scan.in_system_turn = false;
        scan.awaiting_user_response = true;
        scan.saw_user_input = true;
        scan.latest_completed = false;
        scan.has_interactive_ask = false;
    } else if source == Some("MODEL") {
        if is_completion_entry(line) {
            scan.latest_completed = true;
            scan.awaiting_user_response = false;
            scan.has_interactive_ask = entry_has_interactive_tool(&val);
        } else {
            scan.latest_completed = false;
            scan.has_interactive_ask = false;
        }
    }
}

/// Scans complete JSONL lines from `turn_start_pos` and computes the trailing turn state.
pub(crate) fn scan_turn_slice(path: &Path, turn_start_pos: u64) -> TurnSliceScan {
    let Ok(meta) = std::fs::metadata(path) else {
        return TurnSliceScan { next_pos: turn_start_pos, ..Default::default() };
    };
    let start = if meta.len() < turn_start_pos { 0 } else { turn_start_pos };
    if meta.len() <= start {
        return TurnSliceScan { next_pos: start, ..Default::default() };
    }
    let Ok(mut f) = std::fs::File::open(path) else {
        return TurnSliceScan { next_pos: start, ..Default::default() };
    };
    let mut buf = Vec::new();
    if f.seek(SeekFrom::Start(start)).is_err() || f.read_to_end(&mut buf).is_err() {
        return TurnSliceScan { next_pos: start, ..Default::default() };
    }
    let Some(last_nl) = buf.iter().rposition(|&b| b == b'\n') else {
        return TurnSliceScan { next_pos: start, ..Default::default() };
    };
    let mut scan = TurnSliceScan { next_pos: start + last_nl as u64 + 1, ..Default::default() };
    for line in String::from_utf8_lossy(&buf[..=last_nl]).lines().map(str::trim).filter(|l| !l.is_empty()) {
        apply_scan_line(&mut scan, line);
    }
    scan
}

/// Checks if `path` was modified recently (`<120s`) and ends on a non-terminal in-progress step.
pub(crate) fn has_unsettled_turn_on_disk(path: &Path) -> Option<u64> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() == 0 {
        return None;
    }
    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    if now.saturating_sub(mtime) > 120 {
        return None;
    }
    let seek_pos = meta.len().saturating_sub(32 * 1024);
    let mut f = std::fs::File::open(path).ok()?;
    f.seek(SeekFrom::Start(seek_pos)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let slice = if seek_pos > 0 {
        let first_nl = buf.iter().position(|&b| b == b'\n')?;
        (&buf[first_nl + 1..], seek_pos + first_nl as u64 + 1)
    } else {
        (&buf[..], 0)
    };
    let text = String::from_utf8_lossy(slice.0);
    let last = text.lines().rev().map(str::trim).find(|l| !l.is_empty())?;
    let val: serde_json::Value = serde_json::from_str(last).ok()?;
    let etype = val.get("type").and_then(|s| s.as_str());
    let source = val.get("source").and_then(|s| s.as_str());
    let unsettled = matches!(etype, Some("SYSTEM_MESSAGE" | "USER_INPUT"))
        || (source == Some("MODEL") && !is_completion_entry(last) && !is_thinking_only_entry(last));
    unsettled.then_some(slice.1)
}

fn ingest_answer_line(
    line: &str,
    sys_list: &mut Vec<String>,
    cur_sys: &mut Option<String>,
    usr_ans: &mut Option<String>,
    state: &mut (bool, bool),
) {
    let (in_sys, awaiting_usr) = state;
    let Ok(serde_json::Value::Object(obj)) = serde_json::from_str(line) else { return };
    match obj.get("type").and_then(|v| v.as_str()) {
        Some("SYSTEM_MESSAGE") if !*awaiting_usr => {
            if let Some(s) = cur_sys.take() { sys_list.push(s); }
            *in_sys = true;
        }
        Some("USER_INPUT") => {
            if let Some(s) = cur_sys.take() { sys_list.push(s); }
            *in_sys = false;
            *awaiting_usr = true;
            *usr_ans = None;
        }
        Some("PLANNER_RESPONSE")
            if obj.get("source").and_then(|v| v.as_str()) == Some("MODEL")
                && obj.get("status").and_then(|v| v.as_str()) == Some("DONE") =>
        {
            let no_tools = obj.get("tool_calls").and_then(|v| v.as_array()).is_none_or(|a| a.is_empty());
            if no_tools {
                *awaiting_usr = false;
                if let Some(c) = obj.get("content").and_then(|v| v.as_str()).map(str::trim)
                    && !c.is_empty()
                    && !is_placeholder_content(c)
                {
                    if *in_sys { *cur_sys = Some(c.to_string()); } else { *usr_ans = Some(c.to_string()); }
                }
            }
        }
        _ => {}
    }
}

/// Extracts `(system_turn_answers, final_user_turn_answer)` from `[start_pos .. EOF]`.
pub(crate) fn read_slice_answers(path: &Path, start_pos: u64) -> (Option<String>, Option<String>) {
    let Ok(mut f) = std::fs::File::open(path) else { return (None, None) };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let pos = if len < start_pos { 0 } else { start_pos };
    let mut buf = Vec::new();
    if f.seek(SeekFrom::Start(pos)).is_err() || f.read_to_end(&mut buf).is_err() {
        return (None, None);
    }
    let (mut sys_list, mut cur_sys, mut usr_ans, mut state) = (Vec::new(), None, None, (false, false));
    for line in String::from_utf8_lossy(&buf).lines().map(str::trim).filter(|l| !l.is_empty()) {
        ingest_answer_line(line, &mut sys_list, &mut cur_sys, &mut usr_ans, &mut state);
    }
    if let Some(s) = cur_sys { sys_list.push(s); }
    ((!sys_list.is_empty()).then(|| sys_list.join("\n\n")), usr_ans)
}

/// Waits for any active background async turn on `sid` / `tp` to settle before writing `prompt`.
pub(crate) async fn settle_before_pty_prompt(
    sessions: &SessionManager,
    sid: &str,
    tp: &Path,
    prompt: &str,
) -> Result<Option<String>, String> {
    if prompt.is_empty() || sessions.was_recently_interrupted(sid, Duration::from_secs(120)).await {
        return Ok(None);
    }
    let t0 = Instant::now();
    while sessions.is_async_running(sid).await && t0.elapsed() < Duration::from_secs(30) {
        if sessions.is_interrupted(sid).await {
            sessions.clear_interrupted(sid).await;
            return Err("Interrupted by user (/stop)".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let Some(bg_start) = has_unsettled_turn_on_disk(tp) else {
        return Ok(None);
    };
    let (w0, mut quiet_since) = (Instant::now(), Instant::now());
    let mut prev_tp = std::fs::metadata(tp).map(|m| m.len()).unwrap_or(0);
    let mut prev_pty = sessions.get_output_len(sid).await.unwrap_or(0);
    while has_unsettled_turn_on_disk(tp).is_some() && w0.elapsed() < Duration::from_secs(60) {
        if sessions.is_interrupted(sid).await {
            sessions.clear_interrupted(sid).await;
            return Err("Interrupted by user (/stop)".into());
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
        let cur_tp = std::fs::metadata(tp).map(|m| m.len()).unwrap_or(prev_tp);
        let cur_pty = sessions.get_output_len(sid).await.unwrap_or(prev_pty);
        if cur_tp != prev_tp || cur_pty != prev_pty {
            (prev_tp, prev_pty, quiet_since) = (cur_tp, cur_pty, Instant::now());
        } else if quiet_since.elapsed() >= Duration::from_millis(1500) {
            return Ok(None);
        }
    }
    tokio::time::sleep(Duration::from_millis(120)).await;
    let (sys_a, usr_a) = read_slice_answers(tp, bg_start);
    Ok(usr_a.or(sys_a))
}

/// Combines any unobserved pre-settled or mid-slice background answer with the user turn's answer.
pub(crate) fn combine_turn_answers(
    tp: &Path,
    turn_start_pos: u64,
    pre_settled_bg: Option<String>,
    fallback: Option<String>,
) -> Option<String> {
    let (in_sys, in_usr) = read_slice_answers(tp, turn_start_pos);
    let mut parts: Vec<String> = Vec::new();
    for cand in [pre_settled_bg, in_sys, in_usr.or(fallback)] {
        if let Some(txt) = cand.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
            && !parts.iter().any(|p| p == &txt)
        {
            parts.push(txt);
        }
    }
    (!parts.is_empty()).then(|| parts.join("\n\n---\n\n"))
}
