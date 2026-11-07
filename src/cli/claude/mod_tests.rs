#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::cli::antigravity::AntigravityCli;
    use crate::cli::{AgentProvider, StreamEvent};
    use crate::config::CliConfig;
    use futures::StreamExt;
    use std::path::Path;

    #[test]
    fn test_resolve_claude_model_effort_variants() {
        assert_eq!(
            resolve_claude_model_effort(Some("claude-fable-5"), Some("xhigh")),
            (Some("claude-fable-5".to_string()), Some("xhigh".to_string()))
        );
        assert_eq!(
            resolve_claude_model_effort(Some("claude-sonnet-5-max"), None),
            (Some("claude-sonnet-5".to_string()), Some("max".to_string()))
        );
        assert_eq!(
            resolve_claude_model_effort(Some("claude-opus-4-8"), Some("minimal")),
            (Some("claude-opus-4-8".to_string()), Some("low".to_string()))
        );
        assert_eq!(
            resolve_claude_model_effort(Some("gemini-3.8-flash"), Some("high")),
            (None, Some("high".to_string()))
        );
    }

    #[test]
    fn test_parse_claude_models_response_from_initialize() {
        let init_line = r#"{"type":"control_response","response":{"subtype":"success","response":{"models":[{"value":"default","resolvedModel":"claude-opus-4-8[1m]","supportedEffortLevels":["low","medium","high","xhigh","max"]},{"value":"opus[1m]","resolvedModel":"claude-opus-4-8[1m]","supportedEffortLevels":["low","medium","high","xhigh","max"]},{"value":"sonnet","resolvedModel":"claude-sonnet-5","supportedEffortLevels":["low","medium","high","xhigh","max"]},{"value":"haiku","resolvedModel":"claude-haiku-4-5-20251001"}]}}}"#;
        let models = parse_claude_models_response(init_line);
        assert_eq!(models[0], "claude-opus-4-8-low");
        assert_eq!(models[4], "claude-opus-4-8-max");
        assert_eq!(models[5], "claude-sonnet-5-low");
        assert_eq!(models[10], "claude-haiku-4-5");
    }

    #[test]
    fn test_build_claude_command_flags_and_resume() {
        let mut cfg = CliConfig {
            provider: "claude".to_string(),
            model: Some("claude-fable-5".to_string()),
            effort: Some("max".to_string()),
            permission_mode: "bypassPermissions".to_string(),
            ..Default::default()
        };
        cfg.cli_parameters.insert("claude".to_string(), vec!["--bare".to_string()]);
        let cli = AntigravityCli::new(cfg);

        let warm_args = build_warm_command(&cli, Some("sess-uuid-123"), false);
        assert!(warm_args.contains(&"-p".to_string()));
        assert!(warm_args.contains(&"--input-format".to_string()));
        assert!(warm_args.contains(&"--output-format".to_string()));
        assert!(warm_args.contains(&"stream-json".to_string()));
        assert!(warm_args.contains(&"--verbose".to_string()));
        assert!(warm_args.contains(&"--dangerously-skip-permissions".to_string()));
        assert!(warm_args.contains(&"--resume".to_string()));
        assert!(warm_args.contains(&"sess-uuid-123".to_string()));
        assert!(warm_args.contains(&"--model".to_string()));
        assert!(warm_args.contains(&"claude-fable-5".to_string()));
        assert!(warm_args.contains(&"--effort".to_string()));
        assert!(warm_args.contains(&"max".to_string()));
        assert!(warm_args.contains(&"--bare".to_string()));

        let args = build_command(&cli, "Hello Claude", Some("sess-uuid-123"), false);
        assert_eq!(args.last().unwrap(), "Hello Claude");
    }

    #[test]
    fn test_claude_stream_parser_jsonl_events() {
        let mut parser = ClaudeStreamParser::new();

        let ev0 = parser.parse_line(r#"{"type":"system","subtype":"init","session_id":"claude-sid-999"}"#);
        assert!(ev0.is_empty());
        assert_eq!(parser.session_id.as_deref(), Some("claude-sid-999"));

        let ev1 = parser.parse_line(
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"Analyzing request..."},{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#,
        );
        assert_eq!(ev1.len(), 1);
        if let StreamEvent::TextDelta(ref d) = ev1[0] {
            assert!(d.contains("Analyzing request..."));
            assert!(d.contains("🛠 Tool: `Bash`"));
        } else {
            panic!("Expected TextDelta");
        }

        let ev2 = parser.parse_line(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Final Claude answer."}]}}"#,
        );
        assert_eq!(ev2.len(), 1);
        if let StreamEvent::TextDelta(ref d) = ev2[0] {
            assert!(d.contains("Final Claude answer."));
        } else {
            panic!("Expected TextDelta");
        }

        let ev3 = parser.parse_line(
            r#"{"type":"result","subtype":"success","is_error":false,"result":"Final Claude answer.","session_id":"claude-sid-999"}"#,
        );
        assert_eq!(ev3.len(), 1);
        if let StreamEvent::Result(ref r) = ev3[0] {
            assert_eq!(r.session_id.as_deref(), Some("claude-sid-999"));
            assert_eq!(r.result, "Final Claude answer.");
            assert!(!r.is_error);
        } else {
            panic!("Expected Result");
        }
    }

    fn load_gemini_api_key() -> Option<String> {
        if let Ok(k) = std::env::var("GEMINI_API_KEY") && !k.trim().is_empty() {
            return Some(k);
        }
        let home = std::env::var("HOME").ok()?;
        let env_content = std::fs::read_to_string(Path::new(&home).join(".tuner/.env")).ok()?;
        for line in env_content.lines() {
            if let Some(val) = line.trim().strip_prefix("GEMINI_API_KEY=") {
                let clean = val.trim().trim_matches('"').trim_matches('\'');
                if !clean.is_empty() { return Some(clean.to_string()); }
            }
        }
        None
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_live_claude_warm_process_via_gemini_bridge_reuses_pid() {
        let _guard = crate::telegram::TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let bridge_script = Path::new("/home/wimvm/.tuner/profiles/default/workspace/scratch/llm_bridge/gemini_bridge.py");
        let Some(api_key) = load_gemini_api_key() else { return; };
        if !bridge_script.is_file() || !Path::new(&resolve_claude_bin()).exists() && resolve_claude_bin() != "claude" {
            return;
        }

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let mut bridge = tokio::process::Command::new("python3")
            .arg(bridge_script)
            .env("GEMINI_API_KEY", &api_key)
            .env("GEMINI_BRIDGE_PORT", port.to_string())
            .env("GEMINI_BRIDGE_MODEL", "gemma-4-26b-a4b-it")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();

        for _ in 0..30 {
            if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        unsafe {
            std::env::set_var("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{}", port));
            std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-bridge-test");
        }

        let temp_ws = tempfile::tempdir().unwrap();
        let mut cfg = CliConfig {
            chat_id: 77701,
            topic_id: Some(1),
            provider: "claude".to_string(),
            model: Some("claude-fable-5".to_string()),
            effort: Some("xhigh".to_string()),
            permission_mode: "bypassPermissions".to_string(),
            working_dir: temp_ws.path().to_path_buf(),
            ..Default::default()
        };
        cfg.cli_parameters.insert("claude".to_string(), vec!["--bare".to_string()]);
        let mut cli = AntigravityCli::new(cfg);

        let mut stream = cli
            .send_streaming("Reply with the exact token CLAUDE_TURN1_OK and nothing else.", None, false, temp_ws.path().to_path_buf())
            .await
            .unwrap();
        let mut deltas = Vec::new();
        let mut turn1_res = None;
        while let Some(ev) = stream.next().await {
            match ev {
                StreamEvent::TextDelta(d) => deltas.push(d),
                StreamEvent::Result(r) => turn1_res = Some(r),
                _ => {}
            }
        }
        drop(stream);
        let r1 = turn1_res.expect("Expected Result from claude turn 1");
        assert!(!r1.is_error, "Turn 1 error: {} / stderr: {}", r1.result, r1.stderr);
        assert!(!deltas.is_empty(), "Expected streaming TextDelta events from claude");
        assert!(r1.result.contains("CLAUDE_TURN1_OK"), "Unexpected result: {}", r1.result);
        let sid1 = r1.session_id.expect("Expected session_id from claude turn 1");
        let pid1 = active_pid(77701, Some(1)).await.expect("Warm claude process should remain alive after Turn 1");

        assert!(set_model_live(77701, Some(1), Some(&sid1), "claude-sonnet-5").await);
        cli.config.model = Some("claude-sonnet-5".to_string());
        let r2 = cli
            .send("Now reply with the exact token CLAUDE_TURN2_OK and nothing else.", Some(&sid1), false, temp_ws.path().to_path_buf())
            .await
            .unwrap();
        assert!(!r2.is_error, "Turn 2 error: {} / stderr: {}", r2.result, r2.stderr);
        assert_eq!(r2.session_id.as_deref(), Some(sid1.as_str()));
        assert!(r2.result.contains("CLAUDE_TURN2_OK"), "Unexpected turn 2 result: {}", r2.result);
        let pid2 = active_pid(77701, Some(1)).await.expect("Warm claude process should remain alive after Turn 2");
        assert_eq!(pid1, pid2, "Expected Turn 1 and Turn 2 to reuse the exact same warm claude PID");

        cli.config.effort = Some("max".to_string());
        let r3 = cli
            .send("Now reply with the exact token CLAUDE_TURN3_OK and nothing else.", Some(&sid1), false, temp_ws.path().to_path_buf())
            .await
            .unwrap();
        assert!(!r3.is_error, "Turn 3 error: {} / stderr: {}", r3.result, r3.stderr);
        assert_eq!(r3.session_id.as_deref(), Some(sid1.as_str()));
        let pid3 = active_pid(77701, Some(1)).await.expect("Warm claude process should remain alive after Turn 3");
        assert_ne!(pid2, pid3, "Expected effort change to respawn warm claude process with --resume and new --effort");

        assert!(terminate_topic(77701, Some(1), Some(&sid1)).await);
        assert!(active_pid(77701, Some(1)).await.is_none());
        let _ = bridge.kill().await;
    }
}
