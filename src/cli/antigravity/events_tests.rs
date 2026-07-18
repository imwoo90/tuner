use super::events::*;
use super::log_parser::AntigravityLogParser;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[test]
fn test_antigravity_batch_json_extracts_common_content_keys() {
    assert_eq!(parse_antigravity_json("{\"result\":\"ok\"}"), "ok");
    assert_eq!(parse_antigravity_json("{\"content\":\"hello\"}"), "hello");
    assert_eq!(parse_antigravity_json("{\"text\":\"world\"}"), "world");
    assert_eq!(parse_antigravity_json("{\"message\":\"hi\"}"), "hi");
    assert_eq!(parse_antigravity_json("plain"), "plain");
    assert_eq!(parse_antigravity_json(""), "");
}

fn create_test_dir(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("target");
    p.push("test_dirs");
    p.push(name);
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn write_transcript(root: &Path, conv_id: &str, entries: Vec<serde_json::Value>) {
    let logs = root.join("brain").join(conv_id).join(".system_generated").join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let content = entries
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<String>>()
        .join("\n");
    std::fs::write(logs.join("transcript_full.jsonl"), content).unwrap();
}

fn map_cwd(root: &Path, cwd: &Path, conv_id: &str) {
    let cache = root.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    let mapping = serde_json::json!({
        cwd.to_string_lossy().to_string(): conv_id
    });
    std::fs::write(cache.join("last_conversations.json"), mapping.to_string()).unwrap();
}

#[test]
fn test_returns_last_planner_response() {
    let root = create_test_dir("returns_last_planner_response");
    let cwd = root.join("ws");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut env = HashMap::new();
    env.insert("HOME".to_string(), root.to_string_lossy().to_string());

    let state_root = agy_state_root(Some(&env));
    map_cwd(&state_root, &cwd, "conv-1");
    write_transcript(
        &state_root,
        "conv-1",
        vec![
            serde_json::json!({
                "source": "USER_EXPLICIT",
                "type": "USER_INPUT",
                "status": "DONE",
                "content": "hi"
            }),
            serde_json::json!({
                "source": "MODEL",
                "type": "PLANNER_RESPONSE",
                "status": "DONE",
                "content": "intermediate plan"
            }),
            serde_json::json!({
                "source": "MODEL",
                "type": "LIST_DIRECTORY",
                "status": "DONE",
                "content": "ls"
            }),
            serde_json::json!({
                "source": "MODEL",
                "type": "PLANNER_RESPONSE",
                "status": "DONE",
                "content": "final answer"
            }),
        ],
    );

    assert_eq!(
        read_transcript_answer(&cwd, Some(&env), None),
        Some("final answer".to_string())
    );
}

#[test]
fn test_ignores_tool_steps_only() {
    let root = create_test_dir("ignores_tool_steps_only");
    let cwd = root.join("ws");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut env = HashMap::new();
    env.insert("HOME".to_string(), root.to_string_lossy().to_string());

    let state_root = agy_state_root(Some(&env));
    map_cwd(&state_root, &cwd, "conv-1");
    write_transcript(
        &state_root,
        "conv-1",
        vec![serde_json::json!({
            "source": "MODEL",
            "type": "LIST_DIRECTORY",
            "status": "DONE",
            "content": "ls"
        })],
    );

    assert_eq!(read_transcript_answer(&cwd, Some(&env), None), None);
}

#[test]
fn test_falls_back_to_newest_brain_when_cwd_unmapped() {
    let root = create_test_dir("falls_back_to_newest");
    let cwd = root.join("unmapped");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut env = HashMap::new();
    env.insert("HOME".to_string(), root.to_string_lossy().to_string());

    let state_root = agy_state_root(Some(&env));
    write_transcript(
        &state_root,
        "conv-x",
        vec![serde_json::json!({
            "source": "MODEL",
            "type": "PLANNER_RESPONSE",
            "status": "DONE",
            "content": "from newest"
        })],
    );

    assert_eq!(
        read_transcript_answer(&cwd, Some(&env), None),
        Some("from newest".to_string())
    );
}

#[test]
fn test_returns_none_without_state() {
    let root = create_test_dir("without_state");
    let cwd = root.join("ws");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut env = HashMap::new();
    env.insert("HOME".to_string(), root.to_string_lossy().to_string());

    assert_eq!(read_transcript_answer(&cwd, Some(&env), None), None);
}

#[test]
fn test_prefers_userprofile_then_home() {
    let mut env = HashMap::new();
    env.insert("USERPROFILE".to_string(), "/win/home".to_string());
    env.insert("HOME".to_string(), "/unix/home".to_string());
    assert_eq!(
        agy_state_root(Some(&env)),
        PathBuf::from("/win/home/.gemini/antigravity-cli")
    );

    let mut env2 = HashMap::new();
    env2.insert("HOME".to_string(), "/unix/home".to_string());
    assert_eq!(
        agy_state_root(Some(&env2)),
        PathBuf::from("/unix/home/.gemini/antigravity-cli")
    );
}

#[test]
fn test_falls_back_to_user_home() {
    let env = HashMap::new();
    let state_root = agy_state_root(Some(&env));
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let expected = PathBuf::from(home)
        .join(".gemini")
        .join("antigravity-cli");
    assert_eq!(state_root, expected);
}


#[test]
fn test_log_parser_initializes_to_file_size() {
    let root = create_test_dir("log_parser_init");
    let transcript = root.join("transcript_full.jsonl");
    std::fs::write(&transcript, "{\"dummy\":true}\n").unwrap();
    let initial_size = transcript.metadata().unwrap().len();

    let mut parser = AntigravityLogParser::new();
    let (next_size, delta, _) = parser.parse_log_delta(&transcript, None);
    assert_eq!(next_size, initial_size);
    assert!(delta.is_none());
}

#[test]
fn test_log_parser_first_read_with_content() {
    let root = create_test_dir("log_parser_first_read");
    let transcript = root.join("transcript_full.jsonl");
    let entry = serde_json::json!({
        "source": "MODEL",
        "type": "PLANNER_RESPONSE",
        "thinking": "Initial thought process",
    });
    std::fs::write(&transcript, entry.to_string() + "\n").unwrap();

    let mut parser = AntigravityLogParser::new();
    let (next_size, delta, _) = parser.parse_log_delta(&transcript, None);
    assert!(next_size > 0);
    assert!(delta.is_some());
    let text = delta.unwrap();
    assert!(text.contains("Initial thought process"));
}

#[test]
fn test_log_parser_extracts_thinking_and_tools() {
    let root = create_test_dir("log_parser_extracts");
    let transcript = root.join("transcript_full.jsonl");
    std::fs::write(&transcript, "").unwrap();

    let mut parser = AntigravityLogParser::new();
    
    let entry = serde_json::json!({
        "source": "MODEL",
        "type": "PLANNER_RESPONSE",
        "thinking": "Let me think\nstep 2",
        "tool_calls": [
            {
                "name": "run_command",
                "args": {
                    "CommandLine": "echo hello",
                    "CodeContent": "lots of code..."
                }
            }
        ]
    });
    std::fs::write(&transcript, entry.to_string() + "\n").unwrap();
    
    let (next_size, delta, _) = parser.parse_log_delta(&transcript, Some(0));
    assert!(next_size > 0);
    assert!(delta.is_some());
    let text = delta.unwrap();
    
    assert!(text.contains("💭 **Thinking Process:**"));
    assert!(text.contains(">! Let me think"));
    assert!(text.contains(">! step 2"));
    
    assert!(text.contains("🛠️ **Tool Calls:**"));
    assert!(text.contains("`run_command("));
    assert!(text.contains("CommandLine"));
    assert!(text.contains("<omitted...>"));
}

#[test]
fn test_log_parser_extracts_tool_completions_and_final_response() {
    let root = create_test_dir("log_parser_completions");
    let transcript = root.join("transcript_full.jsonl");
    std::fs::write(&transcript, "").unwrap();

    let mut parser = AntigravityLogParser::new();
    
    let entries = vec![
        serde_json::json!({
            "source": "MODEL",
            "type": "RUN_COMMAND",
            "status": "DONE"
        }),
        serde_json::json!({
            "source": "MODEL",
            "type": "PLANNER_RESPONSE",
            "status": "DONE",
            "content": "All done!"
        })
    ];
    let content = entries.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n") + "\n";
    std::fs::write(&transcript, content).unwrap();
    
    let (next_size, delta, _) = parser.parse_log_delta(&transcript, Some(0));
    assert!(next_size > 0);
    assert!(delta.is_some());
    let text = delta.unwrap();
    
    assert!(text.contains("📥 **Tool Completions:**"));
    assert!(text.contains("`run_command (execute)` completed"));
    assert!(text.contains("✅ **Final Response:**"));
    assert!(text.contains("All done!"));
}

