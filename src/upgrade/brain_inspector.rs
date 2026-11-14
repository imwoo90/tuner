//! # Antigravity Subagent, Native Swarm & Background Task Brain Inspector
//!
//! ## Overview
//! Walks live Antigravity conversation IDs, recursive `SUBAGENT_STATE_ALIVE` child
//! subagent descriptors, and `agy 1.5.7+` Native Swarm `swarm/members.json` rosters
//! on disk, streaming compact JSONL steps in monotonic `step_index` order to detect
//! mid-turn execution, unstarted working swarm members, parked `swarm_wait` turns,
//! live timers, and sentinel crons while excluding quiet `HeartbeatScheduler` turns.
//!
//! ## Search Tags
//! #subagent-tree, #native-swarm, #brain-transcript, #background-tasks, #quiet-heartbeat

use super::step_parser::{
    parse_compact_step_with_ack, update_terminal_state, CompactStep, SwarmBatchState,
};
use super::task_tracker::TaskTracker;
use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::time::UNIX_EPOCH;

fn is_safe_conv_id(cid: &str) -> bool {
    !cid.is_empty()
        && cid != "."
        && !cid.contains('/')
        && !cid.contains('\\')
        && !cid.contains("..")
}

fn file_mtime_epoch(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

fn resolve_transcript_path(brain_root: &Path, cid: &str) -> std::path::PathBuf {
    let logs = brain_root.join(cid).join(".system_generated").join("logs");
    let compact = logs.join("transcript.jsonl");
    if compact.is_file() { compact } else { logs.join("transcript_full.jsonl") }
}

fn has_any_transcript(brain_root: &Path, cid: &str) -> bool {
    resolve_transcript_path(brain_root, cid).is_file()
}

fn push_alive_subagents(
    brain_root: &Path,
    gen_dir: &Path,
    now_epoch: u64,
    stack: &mut Vec<String>,
) -> (bool, u64) {
    let Ok(entries) = std::fs::read_dir(gen_dir.join("subagents")) else { return (false, 0) };
    let mut pre_active = false;
    let mut max_m = 0_u64;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else { continue };
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) else { continue };
        if val.get("state").and_then(|v| v.as_str()) == Some("SUBAGENT_STATE_ALIVE")
            && let Some(cid) = val.get("conversationId").and_then(|v| v.as_str())
            && is_safe_conv_id(cid)
        {
            stack.push(cid.to_string());
            let dm = file_mtime_epoch(&path).unwrap_or(0);
            if !has_any_transcript(brain_root, cid) && now_epoch.saturating_sub(dm) < 20 * 60 {
                pre_active = true;
                max_m = max_m.max(dm);
            }
        }
    }
    (pre_active, max_m)
}

fn is_swarm_member_killed(m: &serde_json::Value) -> bool {
    m.get("killed").and_then(|v| v.as_bool()).unwrap_or(false)
        || m.get("desired_state").and_then(|v| v.as_str()) == Some("KILLED")
        || m.get("phase").and_then(|v| v.as_str()) == Some("KILLED")
        || m.get("state").and_then(|v| v.as_str()) == Some("killed")
}

fn push_swarm_members(
    brain_root: &Path,
    gen_dir: &Path,
    now_epoch: u64,
    stack: &mut Vec<String>,
) -> (bool, bool, u64) {
    let members_path = gen_dir.join("swarm").join("members.json");
    let Ok(raw) = std::fs::read_to_string(&members_path) else { return (false, false, 0) };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) else { return (false, false, 0) };
    let Some(members) = val.get("members").and_then(|v| v.as_object()) else { return (false, false, 0) };
    let members_mtime = file_mtime_epoch(&members_path).unwrap_or(0);
    let mut pre_active = false;
    let mut has_live_member = false;
    let mut max_m = 0_u64;
    for m in members.values().filter(|m| !is_swarm_member_killed(m)) {
        has_live_member = true;
        let Some(cid) = m
            .get("conversation_id")
            .or_else(|| m.get("conversationId"))
            .and_then(|v| v.as_str())
            .filter(|c| is_safe_conv_id(c))
        else {
            continue;
        };
        stack.push(cid.to_string());
        if !has_any_transcript(brain_root, cid) {
            let parked = matches!(m.get("wake_mode").and_then(|v| v.as_str()), Some("mentions" | "anything"))
                || matches!(m.get("state").and_then(|v| v.as_str()), Some("waiting" | "stopped"));
            if !parked && now_epoch.saturating_sub(members_mtime) < 20 * 60 {
                pre_active = true;
                max_m = max_m.max(members_mtime);
            }
        }
    }
    (pre_active, has_live_member, max_m)
}

fn collect_alive_subagent_tree(
    brain_root: &Path,
    roots: &[String],
    now_epoch: u64,
) -> (Vec<String>, bool, bool, u64) {
    let mut visited = HashSet::new();
    let mut stack: Vec<String> = roots
        .iter()
        .filter(|s| is_safe_conv_id(s))
        .cloned()
        .collect();
    let mut ordered = Vec::new();
    let mut pre_active = false;
    let mut has_swarm = false;
    let mut max_m = 0_u64;
    while let Some(sid) = stack.pop() {
        if !is_safe_conv_id(&sid) || !visited.insert(sid.clone()) {
            continue;
        }
        ordered.push(sid.clone());
        let gen_dir = brain_root.join(&sid).join(".system_generated");
        let (sub_act, sub_m) = push_alive_subagents(brain_root, &gen_dir, now_epoch, &mut stack);
        let (sw_act, sw_live, sw_m) = push_swarm_members(brain_root, &gen_dir, now_epoch, &mut stack);
        pre_active |= sub_act || sw_act;
        has_swarm |= sw_live;
        max_m = max_m.max(sub_m).max(sw_m);
    }
    (ordered, pre_active, has_swarm, max_m)
}

fn inspect_transcript_mode(
    path: &Path,
    mtime_epoch: u64,
    now_epoch: u64,
    ack_token: &str,
    include_recent_turn: bool,
) -> (bool, u64) {
    let Ok(file) = std::fs::File::open(path) else {
        return (false, 0);
    };
    let reader = BufReader::with_capacity(64 * 1024, file);
    let mut steps: BTreeMap<(u8, u64), CompactStep> = BTreeMap::new();
    let mut parsed_seq = 0_u64;
    for line_res in reader.lines() {
        let Ok(line) = line_res else { continue };
        if line.trim().is_empty() {
            continue;
        }
        if let Some((key, compact)) =
            parse_compact_step_with_ack(&line, parsed_seq, mtime_epoch, ack_token)
        {
            parsed_seq = parsed_seq.saturating_add(1);
            if compact.stype == "USER_INPUT" && key.0 == 0 && steps.contains_key(&key) {
                let _ = steps.split_off(&key);
            }
            steps.insert(key, compact);
        }
    }
    if steps.is_empty() {
        return (false, 0);
    }
    evaluate_ordered_steps(&steps, mtime_epoch, now_epoch, include_recent_turn)
}

fn evaluate_ordered_steps(
    steps: &BTreeMap<(u8, u64), CompactStep>,
    mtime_epoch: u64,
    now_epoch: u64,
    include_recent_turn: bool,
) -> (bool, u64) {
    let mut tracker = TaskTracker::default();
    let mut last_terminal = true;
    let mut swarm_batch = SwarmBatchState::default();
    let mut saw_hb = false;
    let mut in_hb = false;
    let mut last_quiet_hb = false;
    let mut max_real_ts = 0_u64;
    for (key, step) in steps {
        if step.stype == "USER_INPUT" {
            in_hb = step.is_hb_prompt;
            saw_hb |= in_hb;
            last_quiet_hb = false;
        } else if step.stype == "SYSTEM_MESSAGE" {
            in_hb = false;
            last_quiet_hb = false;
        }
        tracker.ingest_ordered_step(*key, step);
        last_terminal = update_terminal_state(*key, step, &mut swarm_batch);
        if in_hb && step.stype == "PLANNER_RESPONSE" && step.status == "DONE" {
            if step.is_hb_ack {
                last_quiet_hb = true;
            } else if !step.has_tools {
                in_hb = false;
                last_quiet_hb = false;
            }
        }
        if !in_hb {
            max_real_ts = max_real_ts.max(step.step_ts);
        }
    }
    let eff_mtime = if saw_hb { max_real_ts } else { mtime_epoch.max(max_real_ts) };
    let age_secs = now_epoch.saturating_sub(mtime_epoch);
    let mid_turn_active = !in_hb && !last_terminal && age_secs < 20 * 60;
    let recent_real_turn = include_recent_turn && !in_hb && !last_quiet_hb && age_secs < 120;
    let active = tracker.has_live_task(now_epoch) || mid_turn_active || recent_real_turn;
    (active, eff_mtime)
}

/// Returns `true` if `sid` currently has any live or parked Native Swarm member, unstarted `<20m`
/// subagent, live parent background task/timer, or active descendant subagent turn.
pub fn has_live_or_parked_agy_descendants(
    brain_root: &Path,
    sid: &str,
    now_epoch: u64,
    ack_token: &str,
) -> bool {
    if !is_safe_conv_id(sid) || !brain_root.is_dir() {
        return false;
    }
    let (ordered, pre_active, has_swarm, _) =
        collect_alive_subagent_tree(brain_root, &[sid.to_string()], now_epoch);
    if pre_active || has_swarm {
        return true;
    }
    for cid in ordered {
        let tp = resolve_transcript_path(brain_root, &cid);
        if let Some(mtime) = file_mtime_epoch(&tp)
            && inspect_transcript_mode(&tp, mtime, now_epoch, ack_token, cid != sid).0
        {
            return true;
        }
    }
    false
}

/// Inspects `live_sids` and recursive child subagent/swarm conversations under `brain_root`,
/// using custom `ack_token` to exclude quiet heartbeat self-checks from activity timestamps.
pub fn inspect_agy_brain_activity_with_ack(
    brain_root: &Path,
    live_sids: &[String],
    now_epoch: u64,
    ack_token: &str,
) -> (bool, u64) {
    if live_sids.is_empty() || !brain_root.is_dir() {
        return (false, 0);
    }
    let (ordered, mut any_active, _, mut max_mtime) =
        collect_alive_subagent_tree(brain_root, live_sids, now_epoch);
    for cid in ordered {
        let tp = resolve_transcript_path(brain_root, &cid);
        let Some(mtime) = file_mtime_epoch(&tp) else {
            continue;
        };
        let (file_active, eff_mtime) =
            inspect_transcript_mode(&tp, mtime, now_epoch, ack_token, true);
        any_active |= file_active;
        max_mtime = max_mtime.max(eff_mtime);
    }
    (any_active, max_mtime)
}

/// Inspects `live_sids` and recursive child subagent/swarm conversations under `brain_root`,
/// returning `(has_active_work, latest_non_quiet_activity_epoch)`.
pub fn inspect_agy_brain_activity(
    brain_root: &Path,
    live_sids: &[String],
    now_epoch: u64,
) -> (bool, u64) {
    inspect_agy_brain_activity_with_ack(brain_root, live_sids, now_epoch, "HEARTBEAT_OK")
}
