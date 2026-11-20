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

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn test_poll_loop_and_observer_use_single_nonrecursive_inotify_watch() {
        use crate::cli::{CliResponse, StreamEvent};
        use crate::cli::antigravity::polling::spawn_log_polling;
        use crate::messenger::telegram::async_observer::create_brain_dir_watcher;
        use std::collections::HashMap;

        let root = tempdir().unwrap();
        let brain = root.path().join(".gemini/antigravity-cli/brain");
        for s in 0..40 {
            let sid = format!("conv-{s}");
            for step in 0..10 {
                std::fs::create_dir_all(brain.join(&sid).join(".system_generated/steps").join(step.to_string())).unwrap();
            }
            let logs = brain.join(&sid).join(".system_generated/logs");
            std::fs::create_dir_all(&logs).unwrap();
            std::fs::write(logs.join("transcript_full.jsonl"), "").unwrap();
        }

        let count_max_watches_per_fd = || -> usize {
            std::fs::read_dir("/proc/self/fdinfo")
                .unwrap()
                .filter_map(|e| std::fs::read_to_string(e.ok()?.path()).ok())
                .map(|txt| txt.lines().filter(|l| l.starts_with("inotify ")).count())
                .max()
                .unwrap_or(0)
        };

        let (obs_tx, _obs_rx) = tokio::sync::mpsc::unbounded_channel();
        let obs_w = create_brain_dir_watcher(&brain.join("conv-0"), obs_tx);
        assert!(obs_w.is_some());

        let (ev_tx, mut ev_rx) = tokio::sync::mpsc::unbounded_channel::<StreamEvent>();
        let mut env = HashMap::new();
        env.insert("HOME".to_string(), root.path().to_string_lossy().to_string());
        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            let _ = done_rx.await;
            Ok(CliResponse {
                session_id: Some("conv-1".into()),
                result: "ok".into(),
                is_error: false,
                returncode: Some(0),
                stderr: String::new(),
            })
        });
        spawn_log_polling(handle, ev_tx, root.path().to_path_buf(), env, Some(0), Some("conv-1".into()));
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;

        let max_w = count_max_watches_per_fd();
        assert!(max_w <= 2, "Expected <= 2 non-recursive watches per FD across 400+ step dirs, got {}", max_w);

        let _ = done_tx.send(());
        let _ = ev_rx.recv().await;
        drop(obs_w);
    }

    #[tokio::test]
    async fn test_poll_loop_final_drain_emits_ask_question_before_result() {
        use crate::cli::{CliResponse, StreamEvent};
        use crate::cli::antigravity::polling::spawn_log_polling;
        use std::collections::HashMap;

        let root = tempdir().unwrap();
        let sid = "conv-drain-race";
        let logs = root.path().join(".gemini/antigravity-cli/brain").join(sid).join(".system_generated/logs");
        let tp = logs.join("transcript_full.jsonl");

        let mut env = HashMap::new();
        env.insert("HOME".to_string(), root.path().to_string_lossy().to_string());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let tp_bg = tp.clone();
        let logs_bg = logs.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            std::fs::create_dir_all(&logs_bg).unwrap();
            let line = serde_json::json!({
                "source": "MODEL",
                "type": "PLANNER_RESPONSE",
                "status": "DONE",
                "content": "Preamble before ask",
                "tool_calls": [{
                    "name": "ask_question",
                    "args": {
                        "questions": [
                            {"question": "Q1?", "options": ["A", "B"], "is_multi_select": false},
                            {"question": "Q2?", "options": ["C", "D"], "is_multi_select": false}
                        ]
                    }
                }]
            });
            std::fs::write(&tp_bg, format!("{line}\n")).unwrap();
            Ok(CliResponse {
                session_id: Some(sid.into()),
                result: "Preamble before ask".into(),
                is_error: false,
                returncode: Some(0),
                stderr: String::new(),
            })
        });

        // Notice `brain/` did not even exist when `spawn_log_polling` initialized (`watcher == None` initially!),
        // and `oneshot_handle` finishes at 60ms (< 500ms fallback tick), exercising the exact final-drain race!
        spawn_log_polling(handle, tx, root.path().to_path_buf(), env, Some(0), Some(sid.into()));

        let mut events = Vec::new();
        while let Some(ev) = rx.recv().await {
            events.push(ev);
        }
        let ask_idx = events.iter().position(|e| matches!(e, StreamEvent::AskQuestion(qs) if qs.len() == 2));
        let res_idx = events.iter().position(|e| matches!(e, StreamEvent::Result(_)));
        assert!(ask_idx.is_some(), "Expected 2-question AskQuestion event in final drain, got {:?}", events.len());
        assert!(res_idx.is_some(), "Expected Result event");
        assert!(ask_idx.unwrap() < res_idx.unwrap(), "AskQuestion must be emitted before Result");
    }
}

