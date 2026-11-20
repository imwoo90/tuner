//! # Subagent, Native Swarm & Background Task Tree Inspector (`subagent_tree.rs`)
//!
//! ## Overview
//! Queries live warm CLI pool counts and recursively traverses Antigravity
//! `<appDataDir>/brain/<cid>/.system_generated/` artifacts (`subagents/*.json`,
//! `swarm/members.json`, `tasks/*.log`, `logs/transcript.jsonl`) to render a structured
//! live execution tree for `/dashboard/api/subagents`.
//!
//! ## Search Tags
//! #subagent-tree, #native-swarm, #background-tasks, #warm-pids

use super::auth_http::{json_err, require_auth};
use super::{resolve_ctx, WebAppCtx};
use crate::cli::antigravity::events::agy_state_root;
use crate::cli::antigravity::subagent_progress::read_tail_utf8;
use crate::session::key::SessionKey;
use axum::{
    extract::Query,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

/// Subagent descriptor node extracted from `.system_generated/subagents/*.json`.
#[derive(Clone, Debug, Serialize)]
pub struct SubagentDescriptorView {
    /// Child Antigravity conversation ID.
    pub conversation_id: String,
    /// Role or agent label.
    pub role: String,
    /// Lifecycle state string (e.g., `SUBAGENT_STATE_ALIVE`).
    pub state: String,
    /// Descriptor last modification timestamp (Unix seconds).
    pub modified_epoch: u64,
}

/// Native Swarm member entry extracted from `.system_generated/swarm/members.json`.
#[derive(Clone, Debug, Serialize)]
pub struct SwarmMemberView {
    /// Swarm member roster name.
    pub name: String,
    /// Pinned model name, if reported.
    pub model: String,
    /// Current state (`waiting`, `working`, `stopped`, `killed`).
    pub state: String,
    /// Child conversation ID, if spawned.
    pub conversation_id: Option<String>,
}

/// Background task log tail extracted from `.system_generated/tasks/*.log`.
#[derive(Clone, Debug, Serialize)]
pub struct TaskLogView {
    /// Task identifier derived from log filename.
    pub task_id: String,
    /// Last lines from the task log.
    pub tail: String,
}

/// Latest transcript step summary extracted from `logs/transcript.jsonl`.
#[derive(Clone, Debug, Serialize)]
pub struct LatestStepView {
    /// Step index number.
    pub step_index: Option<u64>,
    /// Step type (`USER_INPUT`, `PLANNER_RESPONSE`, `GENERIC`, etc.).
    pub r#type: String,
    /// Step status (`RUNNING`, `DONE`, `ERROR`).
    pub status: String,
    /// Timestamp string if present.
    pub created_at: Option<String>,
    /// Tool names invoked in this step.
    pub tools: Vec<String>,
}

/// Single conversation node in the recursive control center tree.
#[derive(Clone, Debug, Serialize)]
pub struct AgentTreeNode {
    /// Antigravity conversation ID.
    pub conversation_id: String,
    /// True for the session's root conversation.
    pub is_root: bool,
    /// Optional role or swarm roster name for child conversations.
    pub label: Option<String>,
    /// Child subagent descriptors.
    pub subagents: Vec<SubagentDescriptorView>,
    /// Native Swarm roster members.
    pub swarm_members: Vec<SwarmMemberView>,
    /// Background task log tails.
    pub tasks: Vec<TaskLogView>,
    /// Most recent transcript step metadata.
    pub latest_step: Option<LatestStepView>,
}

/// Query parameters for `GET /dashboard/api/subagents`.
#[derive(Deserialize)]
pub struct SubagentsQuery {
    /// Optional target session key.
    #[serde(default)]
    pub session_key: String,
}

fn is_safe_cid(cid: &str) -> bool {
    !cid.is_empty() && cid != "." && !cid.contains('/') && !cid.contains('\\') && !cid.contains("..")
}

fn read_subagents(gen_dir: &Path) -> Vec<SubagentDescriptorView> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(gen_dir.join("subagents")) else {
        return out;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&p) else { continue };
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) else { continue };
        let cid = val.get("conversationId").or_else(|| val.get("conversation_id")).and_then(|v| v.as_str()).unwrap_or("").to_string();
        if !is_safe_cid(&cid) {
            continue;
        }
        let role = val
            .pointer("/subagentDescriptor/role")
            .or_else(|| val.get("role"))
            .or_else(|| val.pointer("/subagentDescriptor/typeName"))
            .or_else(|| val.get("type"))
            .or_else(|| val.get("name"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("subagent")
            .to_string();
        let state = val.get("state").and_then(|v| v.as_str()).unwrap_or("UNKNOWN").to_string();
        let modified_epoch = std::fs::metadata(&p).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
        out.push(SubagentDescriptorView { conversation_id: cid, role, state, modified_epoch });
    }
    out
}

fn read_swarm_members(gen_dir: &Path) -> Vec<SwarmMemberView> {
    let mut out = Vec::new();
    let Ok(raw) = std::fs::read_to_string(gen_dir.join("swarm").join("members.json")) else {
        return out;
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) else { return out };
    let Some(members) = val.get("members").and_then(|v| v.as_object()) else { return out };
    for (k, m) in members {
        let name = m.get("name").and_then(|v| v.as_str()).unwrap_or(k.as_str()).to_string();
        let model = m.get("model").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let state = m.get("state").or_else(|| m.get("phase")).and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
        let cid = m.get("conversation_id").or_else(|| m.get("conversationId")).and_then(|v| v.as_str()).filter(|s| is_safe_cid(s)).map(str::to_string);
        out.push(SwarmMemberView { name, model, state, conversation_id: cid });
    }
    out
}

fn read_tasks(gen_dir: &Path) -> Vec<TaskLogView> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(gen_dir.join("tasks")) else {
        return out;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let task_id = p.file_stem().and_then(|s| s.to_str()).unwrap_or("task").to_string();
        let raw = read_tail_utf8(&p, 8192).unwrap_or_default();
        let lines: Vec<&str> = raw.lines().collect();
        let tail = lines[lines.len().saturating_sub(6)..].join("\n");
        let clipped: String = tail.chars().rev().take(600).collect::<Vec<_>>().into_iter().rev().collect();
        out.push(TaskLogView { task_id, tail: clipped });
    }
    out
}

fn read_latest_step(gen_dir: &Path) -> Option<LatestStepView> {
    let logs = gen_dir.join("logs");
    let compact = logs.join("transcript.jsonl");
    let path = if compact.is_file() { compact } else { logs.join("transcript_full.jsonl") };
    let raw = read_tail_utf8(&path, 32 * 1024)?;
    let last_line = raw.lines().rev().find(|l| !l.trim().is_empty())?;
    let val: serde_json::Value = serde_json::from_str(last_line).ok()?;
    let tools = val
        .get("tool_calls")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(str::to_string)).collect())
        .unwrap_or_default();
    Some(LatestStepView {
        step_index: val.get("step_index").and_then(|v| v.as_u64()),
        r#type: val.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        status: val.get("status").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        created_at: val.get("created_at").and_then(|v| v.as_str()).map(str::to_string),
        tools,
    })
}

/// Traverses `brain_root/<root_cid>/.system_generated/` recursively up to `max_nodes`.
pub fn collect_agent_tree_nodes(brain_root: &Path, root_cid: &str, max_nodes: usize) -> Vec<AgentTreeNode> {
    if !is_safe_cid(root_cid) || !brain_root.is_dir() {
        return Vec::new();
    }
    let mut visited = HashSet::new();
    let mut queue = vec![(root_cid.to_string(), true, None::<String>)];
    let mut nodes = Vec::new();
    while let Some((cid, is_root, label)) = queue.pop() {
        if nodes.len() >= max_nodes || !is_safe_cid(&cid) || !visited.insert(cid.clone()) {
            continue;
        }
        let gen_dir = brain_root.join(&cid).join(".system_generated");
        if !gen_dir.is_dir() && !is_root {
            continue;
        }
        let subagents = read_subagents(&gen_dir);
        let swarm_members = read_swarm_members(&gen_dir);
        for s in &subagents {
            queue.push((s.conversation_id.clone(), false, Some(s.role.clone())));
        }
        for m in &swarm_members {
            if let Some(ref child_cid) = m.conversation_id {
                queue.push((child_cid.clone(), false, Some(format!("🐝 {}", m.name))));
            }
        }
        nodes.push(AgentTreeNode {
            conversation_id: cid,
            is_root,
            label,
            subagents,
            swarm_members,
            tasks: read_tasks(&gen_dir),
            latest_step: read_latest_step(&gen_dir),
        });
    }
    nodes
}

async fn collect_session_tree(ctx: &WebAppCtx, key: &SessionKey) -> (serde_json::Value, Vec<AgentTreeNode>) {
    let Ok(Some(sess)) = ctx.sessions.get_active(key).await else {
        return (serde_json::Value::Null, Vec::new());
    };
    let agy_sid = sess.get_session_id("antigravity");
    let agy_pid = if !agy_sid.is_empty() {
        ctx.cli.sessions.holders.lock().await.get(&agy_sid).and_then(|h| h.child.id())
    } else {
        None
    };
    let cl_pid = crate::cli::claude::active_pid(key.chat_id, key.topic_id).await;
    let cx_pid = crate::cli::codex::active_pid(key.chat_id, key.topic_id).await;
    let pids = serde_json::json!({ "agy": agy_pid, "claude": cl_pid, "codex": cx_pid });
    let nodes = if !agy_sid.is_empty() {
        let root = agy_state_root(Some(&ctx.cli.build_env())).join("brain");
        collect_agent_tree_nodes(&root, &agy_sid, 32)
    } else {
        Vec::new()
    };
    (pids, nodes)
}

/// Returns warm pool stats, session PIDs, and recursive subagent/swarm/task tree JSON.
pub async fn handle_get_subagents(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Query(q): Query<SubagentsQuery>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_auth(&ctx, &headers).await {
        return *r;
    }
    let (agy_cnt, cl_cnt, cx_cnt) = (
        ctx.cli.sessions.active_count().await,
        crate::cli::claude::active_count().await,
        crate::cli::codex::active_count().await,
    );
    let (session_pids, nodes) = if !q.session_key.trim().is_empty() {
        let Ok(key) = q.session_key.parse::<SessionKey>() else {
            return json_err(StatusCode::BAD_REQUEST, "Invalid session_key");
        };
        collect_session_tree(&ctx, &key).await
    } else {
        (serde_json::Value::Null, Vec::new())
    };
    Json(serde_json::json!({
        "warm_pools": { "agy": agy_cnt, "claude": cl_cnt, "codex": cx_cnt, "total_active": agy_cnt + cl_cnt + cx_cnt },
        "session_pids": session_pids,
        "nodes": nodes
    }))
    .into_response()
}
