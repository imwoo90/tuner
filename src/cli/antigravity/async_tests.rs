#[cfg(test)]
mod tests {
    use crate::cli::antigravity::log_parser::AntigravityLogParser;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_consecutive_async_background_turns() {
        let dir = tempdir().unwrap();
        let transcript = dir.path().join("transcript_full.jsonl");
        std::fs::write(&transcript, "").unwrap();

        let mut parser = AntigravityLogParser::new();
        let mut last_size = 0;

        // Turn 1
        let turn1 = serde_json::json!({
            "source": "MODEL",
            "type": "PLANNER_RESPONSE",
            "status": "DONE",
            "content": "Async turn 1"
        }).to_string() + "\n";
        std::fs::write(&transcript, &turn1).unwrap();

        let d1 = parser.parse_log_delta_structured(&transcript, Some(last_size));
        last_size = d1.new_size;
        assert_eq!(d1.formatted.unwrap(), "Async turn 1");

        // Reset parser state as done in check_and_dispatch_delta
        parser = AntigravityLogParser::new();

        // Turn 2
        let turn2 = serde_json::json!({
            "source": "MODEL",
            "type": "PLANNER_RESPONSE",
            "status": "DONE",
            "content": "Async turn 2"
        }).to_string() + "\n";
        let mut file = std::fs::OpenOptions::new().append(true).open(&transcript).unwrap();
        file.write_all(turn2.as_bytes()).unwrap();

        let d2 = parser.parse_log_delta_structured(&transcript, Some(last_size));
        assert_eq!(d2.formatted.unwrap(), "Async turn 2");
    }

    #[test]
    fn test_async_structured_delta_system_message_and_tools() {
        let dir = tempdir().unwrap();
        let transcript = dir.path().join("transcript_full.jsonl");

        let entries = [serde_json::json!({
                "source": "SYSTEM",
                "type": "SYSTEM_MESSAGE",
                "content": "background task completed"
            }),
            serde_json::json!({
                "source": "MODEL",
                "type": "PLANNER_RESPONSE",
                "tool_calls": [{"name": "run_command", "args": {"CommandLine": "cargo test"}}]
            })];
        let content = entries.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n") + "\n";
        std::fs::write(&transcript, content).unwrap();

        let mut parser = AntigravityLogParser::new();
        let delta = parser.parse_log_delta_structured(&transcript, Some(0));
        assert!(delta.has_activity);
        assert!(delta.final_content.is_none());
        assert!(delta.formatted.is_some());
    }

    #[test]
    fn test_async_structured_delta_final_content_only() {
        let dir = tempdir().unwrap();
        let transcript = dir.path().join("transcript_full.jsonl");

        let entry = serde_json::json!({
            "source": "MODEL",
            "type": "PLANNER_RESPONSE",
            "status": "DONE",
            "content": "Task finished successfully!"
        });
        std::fs::write(&transcript, entry.to_string() + "\n").unwrap();

        let mut parser = AntigravityLogParser::new();
        let delta = parser.parse_log_delta_structured(&transcript, Some(0));
        assert!(delta.has_activity);
        assert_eq!(delta.final_content.as_deref(), Some("Task finished successfully!"));
    }

    #[test]
    fn test_log_parser_does_not_accumulate_previous_deltas() {
        let dir = tempdir().unwrap();
        let transcript = dir.path().join("transcript_full.jsonl");
        let step1 = serde_json::json!({
            "source": "MODEL",
            "type": "PLANNER_RESPONSE",
            "thinking": "Step 1 thought",
            "tool_calls": [{"name": "run_command", "args": {"CommandLine": "echo step1"}}]
        });
        std::fs::write(&transcript, step1.to_string() + "\n").unwrap();

        let mut parser = AntigravityLogParser::new();
        let d1 = parser.parse_log_delta_structured(&transcript, Some(0));
        let t1 = d1.formatted.unwrap();
        assert!(t1.contains("Step 1 thought"));
        assert!(t1.contains("echo step1"));

        let step2 = serde_json::json!({
            "source": "MODEL",
            "type": "PLANNER_RESPONSE",
            "thinking": "Step 2 thought",
            "tool_calls": [{"name": "run_command", "args": {"CommandLine": "echo step2"}}]
        });
        let mut f = std::fs::OpenOptions::new().append(true).open(&transcript).unwrap();
        f.write_all((step2.to_string() + "\n").as_bytes()).unwrap();

        let d2 = parser.parse_log_delta_structured(&transcript, Some(d1.new_size));
        let t2 = d2.formatted.unwrap();
        assert!(t2.contains("Step 2 thought"));
        assert!(t2.contains("echo step2"));
        assert!(!t2.contains("Step 1 thought"), "Previous delta thinking must not accumulate");
        assert!(!t2.contains("echo step1"), "Previous delta tool calls must not accumulate");
    }

    #[tokio::test]
    async fn test_wait_for_log_completion_auto_recovers_thinking_only_turn() {
        use crate::cli::antigravity::{events::read_transcript_answer, polling::wait_for_log_completion, session::SessionManager};
        use std::collections::HashMap;
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let tp = dir.path().join("transcript_full.jsonl");
        std::fs::write(&tp, "").unwrap();

        let script = dir.path().join("mock_agy.sh");
        let code = format!(
            r#"#!/bin/sh
printf '%s\n' "? for shortcuts  ctrl+p commands"
while IFS= read -r line; do
  case "$line" in
    *"[SYSTEM RECOVERY]"*)
      printf '%s\n' '{{"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"[SYSTEM RECOVERY]"}}' >> "{tp}"
      printf '%s\n' '{{"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","thinking":" short","content":"Recovered Korean response"}}' >> "{tp}"
      ;;
    *)
      printf '%s\n' '{{"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"user question"}}' >> "{tp}"
      printf '%s\n' '{{"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","thinking":"Deep reasoning without content","content":"\n"}}' >> "{tp}"
      ;;
  esac
done
"#,
            tp = tp.display()
        );
        std::fs::write(&script, code).unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();

        let mgr = SessionManager::new();
        let env = HashMap::new();
        mgr.ensure_session("sid-think-1", dir.path(), &script.to_string_lossy(), &[], &env).await.unwrap();
        mgr.write_to_session("sid-think-1", "first question\r").await.unwrap();

        let start = std::time::Instant::now();
        let res = wait_for_log_completion(&mgr, "sid-think-1", Some(tp.clone()), 0).await;
        assert!(res.is_ok(), "Expected Ok(()), got: {:?}", res);
        assert!(start.elapsed().as_secs() < 10, "Should recover in ~3.5s, not 300s");

        let logs_dir = dir.path().join(".system_generated/logs");
        std::fs::create_dir_all(&logs_dir).unwrap();
        std::fs::copy(&tp, logs_dir.join("transcript_full.jsonl")).unwrap();
        assert_eq!(
            read_transcript_answer(dir.path(), None, Some(dir.path())),
            Some("Recovered Korean response".to_string())
        );
        mgr.terminate("sid-think-1").await;
    }
}

