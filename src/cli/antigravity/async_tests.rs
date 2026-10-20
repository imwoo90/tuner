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

        let (s1, delta1, _) = parser.parse_log_delta(&transcript, Some(last_size));
        last_size = s1;
        assert_eq!(delta1.unwrap(), "Async turn 1");

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

        let (_, delta2, _) = parser.parse_log_delta(&transcript, Some(last_size));
        assert_eq!(delta2.unwrap(), "Async turn 2");
    }

    #[test]
    fn test_async_structured_delta_system_message_and_tools() {
        let dir = tempdir().unwrap();
        let transcript = dir.path().join("transcript_full.jsonl");

        let entries = vec![
            serde_json::json!({
                "source": "SYSTEM",
                "type": "SYSTEM_MESSAGE",
                "content": "background task completed"
            }),
            serde_json::json!({
                "source": "MODEL",
                "type": "PLANNER_RESPONSE",
                "tool_calls": [{"name": "run_command", "args": {"CommandLine": "cargo test"}}]
            })
        ];
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
}

