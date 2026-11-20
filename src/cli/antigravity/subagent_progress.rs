//! # Antigravity Subagent & Native Swarm Live Progress Tracker (`subagent_progress.rs`)
//!
//! ## Overview
//! Discovers active recursive Antigravity subagents (`subagents/*.json`) and working Native Swarm
//! members (`swarm/members.json`) under `<brain_root>/<root_sid>`, tailing incremental child
//! `transcript_full.jsonl` updates without recursive `inotify` watches.
//!
//! ## Search Tags
//! #subagent-progress, #native-swarm, #child-transcript, #async-observer

use super::log_parser::AntigravityLogParser;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Reads up to the trailing `max_bytes` UTF-8 lines of `path` aligned to a newline boundary.
pub fn read_tail_utf8(path: &Path, max_bytes: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(max_bytes);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let slice = if start > 0 {
        buf.iter().position(|&b| b == b'\n').map(|i| &buf[i + 1..]).unwrap_or(&buf[..])
    } else {
        &buf[..]
    };
    Some(String::from_utf8_lossy(slice).into_owned())
}

fn is_safe_cid(cid: &str) -> bool {
    !cid.is_empty() && cid != "." && !cid.contains('/') && !cid.contains('\\') && !cid.contains("..")
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn mtime_secs(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

fn is_swarm_killed(m: &serde_json::Value) -> bool {
    m.get("killed").and_then(|v| v.as_bool()).unwrap_or(false)
        || m.get("desired_state").and_then(|v| v.as_str()) == Some("KILLED")
        || m.get("phase").and_then(|v| v.as_str()) == Some("KILLED")
        || m.get("state").and_then(|v| v.as_str()) == Some("killed")
}

fn collect_subagent_jsons(gen_dir: &Path, out: &mut Vec<(String, String, u64)>) {
    let Ok(entries) = std::fs::read_dir(gen_dir.join("subagents")) else { return };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&p) else { continue };
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) else { continue };
        if val.get("state").and_then(|v| v.as_str()) != Some("SUBAGENT_STATE_ALIVE") {
            continue;
        }
        let cid = val.get("conversationId").or_else(|| val.get("conversation_id")).and_then(|v| v.as_str()).unwrap_or("");
        if is_safe_cid(cid) {
            let role = val
                .pointer("/subagentDescriptor/role")
                .or_else(|| val.get("role"))
                .or_else(|| val.pointer("/subagentDescriptor/typeName"))
                .or_else(|| val.get("type"))
                .or_else(|| val.get("name"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("subagent");
            out.push((cid.to_string(), role.to_string(), mtime_secs(&p).unwrap_or(0)));
        }
    }
}

fn collect_swarm_json(gen_dir: &Path, out: &mut Vec<(String, String, u64)>) {
    let mp = gen_dir.join("swarm").join("members.json");
    let Ok(raw) = std::fs::read_to_string(&mp) else { return };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) else { return };
    let Some(members) = val.get("members").and_then(|v| v.as_object()) else { return };
    let mm = mtime_secs(&mp).unwrap_or(0);
    for (k, m) in members {
        if is_swarm_killed(m) {
            continue;
        }
        let st = m.get("state").or_else(|| m.get("phase")).and_then(|v| v.as_str()).unwrap_or("");
        if matches!(st, "waiting" | "stopped") {
            continue;
        }
        if let Some(cid) = m.get("conversation_id").or_else(|| m.get("conversationId")).and_then(|v| v.as_str()).filter(|s| is_safe_cid(s)) {
            let name = m.get("name").and_then(|v| v.as_str()).unwrap_or(k.as_str());
            out.push((cid.to_string(), format!("🐝 {name}"), mm));
        }
    }
}

fn collect_child_entries(gen_dir: &Path, out: &mut Vec<(String, String, u64)>) {
    collect_subagent_jsons(gen_dir, out);
    collect_swarm_json(gen_dir, out);
}

/// Discovers alive descendant subagent and active swarm conversations `(cid, label, desc_mtime)`.
pub fn discover_active_descendants(
    brain_root: &Path,
    root_sid: &str,
    max_nodes: usize,
) -> Vec<(String, String, u64)> {
    if !is_safe_cid(root_sid) || !brain_root.is_dir() {
        return Vec::new();
    }
    let mut visited = HashSet::from([root_sid.to_string()]);
    let mut queue = vec![root_sid.to_string()];
    let mut result = Vec::new();
    while let Some(cur) = queue.pop() {
        if result.len() >= max_nodes {
            break;
        }
        let gen_dir = brain_root.join(&cur).join(".system_generated");
        let mut children = Vec::new();
        collect_child_entries(&gen_dir, &mut children);
        for (cid, label, dm) in children {
            if result.len() >= max_nodes || !visited.insert(cid.clone()) {
                continue;
            }
            queue.push(cid.clone());
            result.push((cid, label, dm));
        }
    }
    result
}

/// Counts descendants under `root_sid` that have active/recent (`<20m`) subagent or swarm work.
pub fn count_active_descendants(brain_root: &Path, root_sid: &str) -> usize {
    let now = now_secs();
    discover_active_descendants(brain_root, root_sid, 24)
        .into_iter()
        .filter(|(cid, _, dm)| {
            let tp = brain_root.join(cid).join(".system_generated/logs/transcript_full.jsonl");
            let tm = mtime_secs(&tp).unwrap_or(0);
            now.saturating_sub((*dm).max(tm)) < 20 * 60
        })
        .count()
}

/// Stateful incremental byte-offset tracker for descendant subagent/swarm transcripts.
#[derive(Default)]
pub struct SubagentProgressTracker {
    offsets: HashMap<String, u64>,
    cards: BTreeMap<String, String>,
}

impl SubagentProgressTracker {
    /// Creates an empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears rendered progress cards when the parent conversation starts or finishes a turn.
    pub fn reset(&mut self) {
        self.cards.clear();
    }

    /// Polls alive descendant transcripts and returns a combined Markdown card if any child advanced.
    pub fn poll_delta(&mut self, brain_root: &Path, root_sid: &str) -> Option<String> {
        let active = discover_active_descendants(brain_root, root_sid, 16);
        let live_ids: HashSet<&str> = active.iter().map(|(c, _, _)| c.as_str()).collect();
        self.cards.retain(|k, _| live_ids.contains(k.as_str()));
        if active.is_empty() {
            return None;
        }
        let now = now_secs();
        let mut changed = false;
        for (cid, label, _) in &active {
            let tp = brain_root.join(cid).join(".system_generated/logs/transcript_full.jsonl");
            let Ok(meta) = std::fs::metadata(&tp) else { continue };
            let prev = match self.offsets.get(cid).copied() {
                Some(o) => o,
                None => {
                    let tm = mtime_secs(&tp).unwrap_or(0);
                    let init = if now.saturating_sub(tm) > 120 { meta.len() } else { 0 };
                    self.offsets.insert(cid.clone(), init);
                    init
                }
            };
            if meta.len() <= prev {
                self.offsets.insert(cid.clone(), meta.len());
                continue;
            }
            let delta = AntigravityLogParser::new().parse_log_delta_structured(&tp, Some(prev));
            self.offsets.insert(cid.clone(), delta.new_size);
            if let Some(fmt) = delta.formatted.filter(|s| !s.trim().is_empty()) {
                let short: String = cid.chars().take(8).collect();
                self.cards.insert(cid.clone(), format!("🤖 **[Subagent: {label} `#{short}`]**\n{fmt}"));
                changed = true;
            }
        }
        (changed && !self.cards.is_empty()).then(|| self.cards.values().cloned().collect::<Vec<_>>().join("\n\n"))
    }
}
