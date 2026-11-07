#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::cli::antigravity::AntigravityCli;
    use crate::cli::{AgentProvider, StreamEvent};
    use crate::config::CliConfig;
    use futures::StreamExt;
    use std::path::Path;

    #[test]
    fn test_resolve_codex_model_effort_variants() {
        assert_eq!(
            resolve_codex_model_effort(Some("gpt-6-astra"), Some("xhigh")),
            (Some("gpt-6-astra".to_string()), Some("xhigh".to_string()))
        );
        assert_eq!(
            resolve_codex_model_effort(Some("gpt-6.1-sol-minimal"), None),
            (Some("gpt-6.1-sol".to_string()), Some("minimal".to_string()))
        );
        assert_eq!(
            resolve_codex_model_effort(Some("gpt-6-astra"), Some("max")),
            (Some("gpt-6-astra".to_string()), Some("xhigh".to_string()))
        );
        assert_eq!(
            resolve_codex_model_effort(Some("gemini-3.8-flash"), Some("high")),
            (None, Some("high".to_string()))
        );
    }

    #[test]
    fn test_parse_codex_models_response_from_app_server() {
        let rpc_line = r#"{"id":2,"result":{"data":[{"id":"gpt-6.1-sol","model":"gpt-6.1-sol","hidden":false,"supportedReasoningEfforts":[{"reasoningEffort":"low"},{"reasoningEffort":"high"},{"reasoningEffort":"ultra"}]},{"id":"hidden-m","model":"hidden-m","hidden":true}]}}"#;
        let models = parse_codex_models_response(rpc_line);
        assert_eq!(
            models,
            vec!["gpt-6.1-sol-low", "gpt-6.1-sol-high", "gpt-6.1-sol-ultra"]
        );
    }

    #[test]
    fn test_build_codex_command_flags_and_app_server() {
        let mut cfg = CliConfig {
            provider: "codex".to_string(),
            model: Some("gpt-6-astra".to_string()),
            effort: Some("xhigh".to_string()),
            permission_mode: "bypassPermissions".to_string(),
            ..Default::default()
        };
        cfg.cli_parameters.insert(
            "codex".to_string(),
            vec!["-c".to_string(), "openai_base_url=\"http://127.0.0.1:18765\"".to_string(), "--ephemeral".to_string()],
        );
        let cli = AntigravityCli::new(cfg);

        let app_srv = build_app_server_command(&cli);
        assert_eq!(app_srv[1], "app-server");
        assert_eq!(app_srv[2], "--stdio");
        assert!(app_srv.contains(&"-c".to_string()));
        assert!(!app_srv.contains(&"--ephemeral".to_string()));

        let args = build_command(&cli, "Hello Codex", Some("thread-uuid-777"), false);
        assert_eq!(args[1], "exec");
        assert_eq!(args[2], "resume");
        assert_eq!(args[3], "thread-uuid-777");
        assert!(args.contains(&"--json".to_string()));
        assert!(args.contains(&"--skip-git-repo-check".to_string()));
        assert!(args.contains(&"--dangerously-bypass-approvals-and-sandbox".to_string()));
        assert!(args.contains(&"-m".to_string()));
        assert!(args.contains(&"gpt-6-astra".to_string()));
        assert!(args.contains(&"-c".to_string()));
        assert!(args.contains(&"model_reasoning_effort=\"xhigh\"".to_string()));
        assert!(args.contains(&"--ephemeral".to_string()));
        assert_eq!(args.last().unwrap(), "Hello Codex");
    }

    #[test]
    fn test_codex_stream_parser_jsonl_and_jsonrpc_events() {
        let mut parser = CodexStreamParser::new();

        let ev0 = parser.parse_line(r#"{"type":"thread.started","thread_id":"codex-thread-001"}"#);
        assert!(ev0.is_empty());
        assert_eq!(parser.session_id.as_deref(), Some("codex-thread-001"));

        let ev1 = parser.parse_line(
            r#"{"method":"item/completed","params":{"threadId":"codex-thread-001","item":{"id":"item_0","type":"reasoning","text":"Checking workspace files..."}}}"#,
        );
        assert_eq!(ev1.len(), 1);
        if let StreamEvent::TextDelta(ref d) = ev1[0] {
            assert!(d.contains("Checking workspace files..."));
        } else {
            panic!("Expected TextDelta");
        }

        let ev2 = parser.parse_line(
            r#"{"method":"item/started","params":{"item":{"id":"item_1","type":"commandExecution","command":"cargo test"}}}"#,
        );
        assert_eq!(ev2.len(), 1);
        if let StreamEvent::TextDelta(ref d) = ev2[0] {
            assert!(d.contains("cargo test"));
        } else {
            panic!("Expected TextDelta");
        }

        let ev3 = parser.parse_line(
            r#"{"method":"item/agentMessage/delta","params":{"threadId":"codex-thread-001","delta":"All tests passed."}}"#,
        );
        assert_eq!(ev3.len(), 1);
        assert_eq!(parser.assistant_text, "All tests passed.");

        let ev4 = parser.parse_line(
            r#"{"method":"turn/completed","params":{"threadId":"codex-thread-001","turn":{"id":"t1","status":"completed"}}}"#,
        );
        assert!(ev4.is_empty());
        assert!(parser.turn_completed);
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
    async fn test_live_codex_warm_process_via_gemini_bridge_reuses_pid() {
        let _guard = crate::telegram::TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let bridge_script = Path::new("/home/wimvm/.tuner/profiles/default/workspace/scratch/llm_bridge/gemini_bridge.py");
        let Some(api_key) = load_gemini_api_key() else { return; };
        if !bridge_script.is_file() || !Path::new(&resolve_codex_bin()).exists() && resolve_codex_bin() != "codex" {
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
            std::env::set_var("OPENAI_BASE_URL", format!("http://127.0.0.1:{}", port));
            std::env::set_var("OPENAI_API_KEY", "sk-bridge-test");
        }

        let temp_ws = tempfile::tempdir().unwrap();
        let mut cfg = CliConfig {
            chat_id: 77702,
            topic_id: Some(2),
            provider: "codex".to_string(),
            model: Some("gpt-6-astra".to_string()),
            effort: Some("xhigh".to_string()),
            permission_mode: "bypassPermissions".to_string(),
            working_dir: temp_ws.path().to_path_buf(),
            ..Default::default()
        };
        cfg.cli_parameters.insert(
            "codex".to_string(),
            vec![
                "-c".to_string(),
                "model_provider=\"bridge\"".to_string(),
                "-c".to_string(),
                "model_providers.bridge.name=\"Bridge\"".to_string(),
                "-c".to_string(),
                format!("model_providers.bridge.base_url=\"http://127.0.0.1:{}\"", port),
                "-c".to_string(),
                "model_providers.bridge.supports_websockets=false".to_string(),
            ],
        );
        let mut cli = AntigravityCli::new(cfg);

        let mut stream = cli
            .send_streaming("Reply with the exact token CODEX_TURN1_OK and nothing else.", None, false, temp_ws.path().to_path_buf())
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
        let r1 = turn1_res.expect("Expected Result from codex turn 1");
        assert!(!r1.is_error, "Turn 1 error: {} / stderr: {}", r1.result, r1.stderr);
        assert!(!deltas.is_empty(), "Expected streaming TextDelta events from codex");
        assert!(r1.result.contains("CODEX_TURN1_OK"), "Unexpected result: {}", r1.result);
        let sid1 = r1.session_id.expect("Expected thread_id from codex turn 1");
        let pid1 = active_pid(77702, Some(2)).await.expect("Warm codex process should remain alive after Turn 1");

        cli.config.effort = Some("minimal".to_string());
        let r2 = cli
            .send("Now reply with the exact token CODEX_TURN2_OK and nothing else.", Some(&sid1), false, temp_ws.path().to_path_buf())
            .await
            .unwrap();
        assert!(!r2.is_error, "Turn 2 error: {} / stderr: {}", r2.result, r2.stderr);
        assert_eq!(r2.session_id.as_deref(), Some(sid1.as_str()));
        assert!(r2.result.contains("CODEX_TURN2_OK"), "Unexpected turn 2 result: {}", r2.result);
        let pid2 = active_pid(77702, Some(2)).await.expect("Warm codex process should remain alive after Turn 2");
        assert_eq!(pid1, pid2, "Expected Turn 1 and Turn 2 to reuse the exact same warm codex PID");

        let mut p = CodexStreamParser::new();
        p.parse_line(r#"{"method":"item/started","params":{"item":{"type":"agentMessage","text":""}}}"#);
        p.parse_line(r#"{"method":"item/agentMessage/delta","params":{"delta":"First msg"}}"#);
        p.parse_line(r#"{"method":"item/completed","params":{"item":{"type":"agentMessage","text":"First msg"}}}"#);
        p.parse_line(r#"{"method":"item/started","params":{"item":{"type":"agentMessage","text":""}}}"#);
        p.parse_line(r#"{"method":"item/agentMessage/delta","params":{"delta":"Second msg"}}"#);
        assert_eq!(p.assistant_text, "Second msg");
        assert_eq!(resolve_codex_model_effort(Some("astra"), None).0.as_deref(), Some("gpt-6-astra"));
        assert_eq!(resolve_codex_model_effort(Some("sol"), None).0.as_deref(), Some("gpt-6.1-sol"));

        assert!(terminate_topic(77702, Some(2), Some(&sid1)).await);
        assert!(active_pid(77702, Some(2)).await.is_none());
        let _ = bridge.kill().await;
    }
}
