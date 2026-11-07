//! # Peer Provider Session, Trust, and Interactive Protocol Helpers (peer.rs)
//!
//! ## Overview
//! Implements disk session existence verification, workspace auto-trust registration,
//! dynamic slash command discovery parsing, and interactive `AskQuestion` protocol payloads
//! for the `claude` and `codex` peer CLI engines.
//!
//! ## Search Tags
//! #peer-session, #workspace-trust, #claude-commands, #ask-question

use crate::cli::AskQuestionData;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn has_codex_session_jsonl(dir: &Path, suffix: &str, depth: usize) -> bool {
    if depth > 4 || !dir.is_dir() {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return false; };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if has_codex_session_jsonl(&path, suffix, depth + 1) {
                return true;
            }
        } else if path.file_name().and_then(|s| s.to_str()).is_some_and(|n| n.ends_with(suffix)) {
            return true;
        }
    }
    false
}

/// Verifies whether a `claude` or `codex` session ID exists on disk under `<HOME>/.claude/projects`
/// or `<HOME>/.codex/sessions`.
pub fn is_peer_session_alive(
    provider: &str,
    session_id: &str,
    env: &HashMap<String, String>,
) -> bool {
    if session_id.is_empty() {
        return false;
    }
    if session_id.starts_with("mock-") {
        return true;
    }
    let Some(home) = env.get("HOME").cloned().or_else(|| std::env::var("HOME").ok()) else {
        return false;
    };
    let home_path = Path::new(&home);
    match provider {
        "claude" => {
            let projects_dir = home_path.join(".claude").join("projects");
            let file_name = format!("{}.jsonl", session_id);
            std::fs::read_dir(projects_dir)
                .ok()
                .into_iter()
                .flatten()
                .flatten()
                .any(|entry| entry.path().join(&file_name).is_file())
        }
        "codex" => {
            let sessions_dir = home_path.join(".codex").join("sessions");
            let suffix = format!("{}.jsonl", session_id);
            has_codex_session_jsonl(&sessions_dir, &suffix, 0)
        }
        _ => false,
    }
}

/// Automatically registers workspace trust in `<HOME>/.claude.json` (`projects[<canonical_workspace>].hasTrustDialogAccepted = true`)
/// so warm `claude` sessions never block on interactive workspace trust dialogs.
pub fn trust_peer_workspace(provider: &str, workspace: &Path, home_override: Option<PathBuf>) {
    if provider != "claude" {
        return;
    }
    let home = home_override.unwrap_or_else(|| {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
    });
    if !home.is_dir() {
        return;
    }
    let claude_json = home.join(".claude.json");
    let mut root = std::fs::read_to_string(&claude_json)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .filter(|v| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}));
    let ws_key = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf())
        .to_string_lossy()
        .to_string();
    let Some(root_obj) = root.as_object_mut() else { return; };
    let projects = root_obj.entry("projects").or_insert_with(|| serde_json::json!({}));
    if !projects.is_object() {
        *projects = serde_json::json!({});
    }
    let Some(proj_map) = projects.as_object_mut() else { return; };
    let entry = proj_map.entry(ws_key).or_insert_with(|| serde_json::json!({}));
    if !entry.is_object() {
        *entry = serde_json::json!({});
    }
    if entry.get("hasTrustDialogAccepted").and_then(|v| v.as_bool()) == Some(true) {
        return;
    }
    if let Some(obj) = entry.as_object_mut() {
        obj.insert("hasTrustDialogAccepted".to_string(), serde_json::Value::Bool(true));
        if let Ok(serialized) = serde_json::to_string_pretty(&root) {
            let _ = std::fs::write(claude_json, serialized);
        }
    }
}

const ALLOWED_CLAUDE_WORKFLOWS: &[&str] = &[
    "review",
    "security_review",
    "verify",
    "debug",
    "simplify",
    "deep_research",
    "code_review",
    "goal",
    "init",
    "batch",
    "loop",
];

/// Parses the `control_response` line from `claude` `initialize` into curated Telegram-compatible `(command, description)` pairs.
pub fn parse_claude_commands_response(line: &str) -> Vec<(String, String)> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(line.trim()) else { return Vec::new(); };
    let Some(arr) = val
        .get("response")
        .and_then(|r| r.get("response"))
        .and_then(|r| r.get("commands"))
        .and_then(|c| c.as_array())
    else {
        return Vec::new();
    };
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for item in arr {
        let Some(raw_name) = item.get("name").and_then(|v| v.as_str()) else { continue; };
        let cmd = raw_name.trim().trim_start_matches('/').replace('-', "_").to_lowercase();
        if !ALLOWED_CLAUDE_WORKFLOWS.contains(&cmd.as_str()) || seen.contains(&cmd) {
            continue;
        }
        seen.push(cmd.clone());
        let raw_desc = item
            .get("description")
            .and_then(|v| v.as_str())
            .and_then(|d| d.lines().next())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Claude Code workflow command");
        let desc: String = raw_desc.chars().take(100).collect();
        out.push((cmd, desc));
    }
    out
}

fn ensure_write_in_option(options: &mut Vec<String>) {
    if !options.iter().any(|o| o.to_lowercase().contains("write-in") || o.contains("직접 입력")) {
        options.push("Write-in (직접 입력)".to_string());
    }
}

fn extract_question_options(q: &serde_json::Value) -> Vec<String> {
    let mut options: Vec<String> = q
        .get("options")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|o| {
                    o.as_str()
                        .or_else(|| o.get("label").and_then(|v| v.as_str()))
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    ensure_write_in_option(&mut options);
    options
}

/// Parses `AskUserQuestion` tool input from a `claude` `can_use_tool` control request into `AskQuestionData` items.
pub fn parse_claude_ask_questions(input: &serde_json::Value) -> Vec<AskQuestionData> {
    let Some(qs) = input.get("questions").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for q in qs {
        let Some(question) = q.get("question").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let is_multi_select = q
            .get("multiSelect")
            .or_else(|| q.get("is_multi_select"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        out.push(AskQuestionData {
            question: question.to_string(),
            options: extract_question_options(q),
            is_multi_select,
        });
    }
    out
}

/// Builds the `control_response` JSON payload for `claude` `AskUserQuestion` with `updatedInput.answers`.
pub fn build_claude_ask_control_response(
    req_id: &str,
    mut input: serde_json::Value,
    answers: &[String],
) -> serde_json::Value {
    let mut answers_map = serde_json::Map::new();
    if let Some(qs) = input.get("questions").and_then(|v| v.as_array()) {
        for (i, q) in qs.iter().enumerate() {
            let ans = answers.get(i).cloned().unwrap_or_default();
            if let Some(q_str) = q.get("question").and_then(|v| v.as_str()) {
                answers_map.insert(q_str.to_string(), serde_json::Value::String(ans.clone()));
            }
            if let Some(q_id) = q.get("id").and_then(|v| v.as_str()) {
                answers_map.entry(q_id.to_string()).or_insert_with(|| serde_json::Value::String(ans));
            }
        }
    }
    if let Some(obj) = input.as_object_mut() {
        obj.insert("answers".to_string(), serde_json::Value::Object(answers_map));
    }
    serde_json::json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": req_id,
            "response": {
                "behavior": "allow",
                "updatedInput": input
            }
        }
    })
}

/// Parses `item/tool/requestUserInput` params from `codex app-server` into `(Vec<AskQuestionData>, Vec<String>)`.
pub fn parse_codex_ask_questions(params: &serde_json::Value) -> (Vec<AskQuestionData>, Vec<String>) {
    let Some(qs) = params.get("questions").and_then(|v| v.as_array()) else {
        return (Vec::new(), Vec::new());
    };
    let mut out = Vec::new();
    let mut q_ids = Vec::new();
    for (idx, q) in qs.iter().enumerate() {
        let Some(question) = q.get("question").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let qid = q
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("q{}", idx));
        let is_multi_select = q
            .get("multiSelect")
            .or_else(|| q.get("is_multi_select"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        q_ids.push(qid);
        out.push(AskQuestionData {
            question: question.to_string(),
            options: extract_question_options(q),
            is_multi_select,
        });
    }
    (out, q_ids)
}

/// Builds the JSON-RPC response payload for `codex app-server` `item/tool/requestUserInput`.
pub fn build_codex_ask_rpc_response(
    rpc_id: serde_json::Value,
    q_ids: &[String],
    answers: &[String],
) -> serde_json::Value {
    let mut answers_map = serde_json::Map::new();
    for (i, qid) in q_ids.iter().enumerate() {
        let ans = answers.get(i).cloned().unwrap_or_default();
        answers_map.insert(qid.clone(), serde_json::json!({ "answers": [ans] }));
    }
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": rpc_id,
        "result": {
            "answers": answers_map
        }
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_is_peer_session_alive_checks_claude_and_codex_jsonl() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = HashMap::new();
        env.insert("HOME".to_string(), tmp.path().to_string_lossy().to_string());

        assert!(is_peer_session_alive("claude", "mock-123", &env));
        assert!(!is_peer_session_alive("claude", "dead-claude-sid", &env));
        assert!(!is_peer_session_alive("codex", "dead-codex-sid", &env));

        let claude_proj = tmp.path().join(".claude/projects/-tmp-ws");
        std::fs::create_dir_all(&claude_proj).unwrap();
        std::fs::write(claude_proj.join("live-claude-sid.jsonl"), "{}").unwrap();
        assert!(is_peer_session_alive("claude", "live-claude-sid", &env));

        let codex_day = tmp.path().join(".codex/sessions/2026/11/08");
        std::fs::create_dir_all(&codex_day).unwrap();
        std::fs::write(codex_day.join("rollout-2026-11-08-live-codex-sid.jsonl"), "{}").unwrap();
        assert!(is_peer_session_alive("codex", "live-codex-sid", &env));
    }

    #[test]
    fn test_trust_peer_workspace_registers_claude_project_trust() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("my_ws");
        std::fs::create_dir_all(&ws).unwrap();

        trust_peer_workspace("claude", &ws, Some(tmp.path().to_path_buf()));

        let content = std::fs::read_to_string(tmp.path().join(".claude.json")).unwrap();
        let val: serde_json::Value = serde_json::from_str(&content).unwrap();
        let canon = ws.canonicalize().unwrap().to_string_lossy().to_string();
        assert_eq!(
            val["projects"][&canon]["hasTrustDialogAccepted"].as_bool(),
            Some(true)
        );
    }

    #[test]
    fn test_parse_claude_commands_and_ask_payloads() {
        let init_line = r#"{"type":"control_response","response":{"subtype":"success","response":{"commands":[{"name":"deep-research","description":"Deep research harness\nSecond line"},{"name":"heapdump","description":"Dump JS heap"},{"name":"reload-skills","description":"Reload skills"},{"name":"code-review","description":"Review diff"}]}}}"#;
        let cmds = parse_claude_commands_response(init_line);
        assert_eq!(
            cmds,
            vec![
                ("deep_research".to_string(), "Deep research harness".to_string()),
                ("code_review".to_string(), "Review diff".to_string()),
            ]
        );

        let ask_in = serde_json::json!({
            "questions": [{
                "question": "Pick DB?",
                "multiSelect": false,
                "options": [{"label": "SQLite"}, {"label": "Postgres"}]
            }]
        });
        let qs = parse_claude_ask_questions(&ask_in);
        assert_eq!(qs.len(), 1);
        assert_eq!(qs[0].question, "Pick DB?");
        assert_eq!(qs[0].options, vec!["SQLite", "Postgres", "Write-in (직접 입력)"]);

        let resp = build_claude_ask_control_response("req_1", ask_in, &["SQLite".to_string()]);
        assert_eq!(
            resp["response"]["response"]["updatedInput"]["answers"]["Pick DB?"].as_str(),
            Some("SQLite")
        );

        let codex_params = serde_json::json!({
            "questions": [{"id": "q_db", "question": "Pick DB?", "options": [{"label": "SQLite"}]}]
        });
        let (c_qs, q_ids) = parse_codex_ask_questions(&codex_params);
        assert_eq!(c_qs.len(), 1);
        assert_eq!(q_ids, vec!["q_db"]);
        let c_resp = build_codex_ask_rpc_response(serde_json::json!(42), &q_ids, &["SQLite".to_string()]);
        assert_eq!(c_resp["result"]["answers"]["q_db"]["answers"][0].as_str(), Some("SQLite"));
    }
}
