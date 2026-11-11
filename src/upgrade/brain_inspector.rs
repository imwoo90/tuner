//! # Antigravity Subagent & Background Task Brain Inspector
//!
//! ## Overview
//! Walks live Antigravity conversation IDs and their recursive `SUBAGENT_STATE_ALIVE`
//! child subagent descriptors on disk, streaming compact JSONL steps in monotonic
//! `step_index` order to detect mid-turn execution, live timers, and sentinel crons
//! while excluding suppressed `HeartbeatScheduler` self-check turns from idle epochs.
//!
//! ## Search Tags
//! #subagent-tree, #brain-transcript, #background-tasks, #quiet-heartbeat

use super::step_parser::{parse_compact_step_with_ack, CompactStep};
use super::task_tracker::TaskTracker;
use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::time::UNIX_EPOCH;

fn collect_alive_subagent_tree(brain_root: &Path, roots: &[String]) -> Vec<String> {
    let mut visited = HashSet::new();
    let mut stack: Vec<String> = roots.to_vec();
    let mut ordered = Vec::new();
    while let Some(sid) = stack.pop() {
        if sid.is_empty() || !visited.insert(sid.clone()) {
            continue;
        }
        ordered.push(sid.clone());
        let sub_dir = brain_root.join(&sid).join(".system_generated").join("subagents");
        let Ok(entries) = std::fs::read_dir(sub_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) else {
                continue;
            };
            if val.get("state").and_then(|v| v.as_str()) == Some("SUBAGENT_STATE_ALIVE")
                && let Some(cid) = val.get("conversationId").and_then(|v| v.as_str())
            {
                stack.push(cid.to_string());
            }
        }
    }
    ordered
}

fn inspect_transcript_file(
    path: &Path,
    mtime_epoch: u64,
    now_epoch: u64,
    ack_token: &str,
) -> (bool, u64) {
    let Ok(file) = std::fs::File::open(path) else {
        return (false, 0);
    };
    let reader = BufReader::with_capacity(64 * 1024, file);
    let mut steps: BTreeMap<(u8, u64), CompactStep> = BTreeMap::new();
    for (seq, line_res) in reader.lines().enumerate() {
        let Ok(line) = line_res else {
            continue;
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Some((key, compact)) =
            parse_compact_step_with_ack(&line, seq as u64, mtime_epoch, ack_token)
        {
            if compact.stype == "USER_INPUT" && key.0 == 0 && steps.contains_key(&key) {
                let _ = steps.split_off(&key);
            }
            steps.insert(key, compact);
        }
    }
    if steps.is_empty() {
        return (false, 0);
    }
    evaluate_ordered_steps(&steps, mtime_epoch, now_epoch)
}

fn evaluate_ordered_steps(
    steps: &BTreeMap<(u8, u64), CompactStep>,
    mtime_epoch: u64,
    now_epoch: u64,
) -> (bool, u64) {
    let mut tracker = TaskTracker::default();
    let mut last_terminal = true;
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
        last_terminal =
            step.stype == "PLANNER_RESPONSE" && step.status == "DONE" && !step.has_tools;
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
    let eff_mtime = if saw_hb {
        max_real_ts
    } else {
        mtime_epoch.max(max_real_ts)
    };
    let age_secs = now_epoch.saturating_sub(mtime_epoch);
    let mid_turn_active = !in_hb && !last_terminal && age_secs < 20 * 60;
    let recent_real_turn = !in_hb && !last_quiet_hb && age_secs < 120;
    let active = tracker.has_live_task(now_epoch) || mid_turn_active || recent_real_turn;
    (active, eff_mtime)
}

/// Inspects `live_sids` and recursive `SUBAGENT_STATE_ALIVE` child conversations under `brain_root`,
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
    let mut any_active = false;
    let mut max_mtime = 0_u64;
    for cid in collect_alive_subagent_tree(brain_root, live_sids) {
        let logs_dir = brain_root.join(&cid).join(".system_generated").join("logs");
        let tp = {
            let compact = logs_dir.join("transcript.jsonl");
            if compact.is_file() {
                compact
            } else {
                logs_dir.join("transcript_full.jsonl")
            }
        };
        let Ok(meta) = std::fs::metadata(&tp) else {
            continue;
        };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if now_epoch.saturating_sub(mtime) < 12 * 3600 {
            let (file_active, eff_mtime) = inspect_transcript_file(&tp, mtime, now_epoch, ack_token);
            any_active |= file_active;
            max_mtime = max_mtime.max(eff_mtime);
        } else {
            max_mtime = max_mtime.max(mtime);
        }
    }
    (any_active, max_mtime)
}

/// Inspects `live_sids` and recursive `SUBAGENT_STATE_ALIVE` child conversations under `brain_root`,
/// returning `(has_active_work, latest_non_quiet_activity_epoch)`.
pub fn inspect_agy_brain_activity(
    brain_root: &Path,
    live_sids: &[String],
    now_epoch: u64,
) -> (bool, u64) {
    inspect_agy_brain_activity_with_ack(brain_root, live_sids, now_epoch, "HEARTBEAT_OK")
}
