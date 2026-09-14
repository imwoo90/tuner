use crate::cli::antigravity::AntigravityCli;
use crate::config::CliConfig;
use crate::cli::{AgentProvider, StreamEvent};
use std::path::{Path, PathBuf};
use futures::StreamExt;

static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn test_build_env_injects_agent_info() {
    let config = CliConfig {
        provider: "antigravity".to_string(),
        chat_id: 1234,
        transport: "tg".to_string(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let env = cli.build_env();

    assert_eq!(env.get("TUNER_AGENT_NAME").unwrap(), "main");
    assert_eq!(env.get("TUNER_CHAT_ID").unwrap(), "1234");
    assert_eq!(env.get("TUNER_TRANSPORT").unwrap(), "tg");
    assert_eq!(env.get("TERM").unwrap(), "dumb");
    assert_eq!(env.get("NO_COLOR").unwrap(), "1");
    assert!(env.contains_key("TUNER_HOME"));
    assert!(env.contains_key("TUNER_SHARED_MEMORY_PATH"));
}

#[test]
fn test_strip_ansi_removes_color_and_control_codes() {
    use super::provider::strip_ansi;
    let colored = "\x1b[94m> \x1b[mHello \x1b[?25hWorld\x1b[2J";
    let clean = strip_ansi(colored);
    assert_eq!(clean, "> Hello World");
}

fn write_mock_script(dir: &Path, code: &str) {
    let path = dir.join("agy");
    std::fs::write(&path, code).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        let _ = std::fs::set_permissions(&path, perms);
    }
}

fn create_mock_agy(dir: &Path) -> PathBuf {
    let agy_mock_path = dir.join("agy");
    let mock_code = r#"#!/bin/sh
has_print=0
for arg in "$@"; do
  if [ "$arg" = "--print" ]; then
    has_print=1
  fi
done
if [ "$has_print" -eq 1 ]; then
  echo '{"result":"mock success"}'
else
  sleep 10
fi
"#;
    write_mock_script(dir, mock_code);
    agy_mock_path
}

#[tokio::test]
async fn test_send_streaming_delegates_to_send_as_delta_and_result() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let temp_dir = tempfile::tempdir().unwrap();
    create_mock_agy(temp_dir.path());
    
    let mut path_env = std::env::var("PATH").unwrap_or_default();
    path_env = format!("{}:{}", temp_dir.path().to_string_lossy(), path_env);
    unsafe {
        std::env::set_var("PATH", path_env);
        std::env::set_var("HOME", temp_dir.path().to_string_lossy().to_string());
    }
    
    let config = CliConfig {
        provider: "antigravity".to_string(),
        model: Some("antigravity-default".to_string()),
        working_dir: temp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let workspace = temp_dir.path().to_path_buf();
    
    let stream_res = cli.send_streaming("hello mock", None, false, workspace).await;
    assert!(stream_res.is_ok(), "Failed: {:?}", stream_res.err());
    
    let events: Vec<StreamEvent> = stream_res.unwrap().collect().await;
    assert_eq!(events.len(), 2);
    
    match &events[0] {
        StreamEvent::TextDelta(text) => assert_eq!(text, "mock success"),
        _ => panic!("First event must be TextDelta"),
    }
    
    match &events[1] {
        StreamEvent::Result(resp) => {
            assert_eq!(resp.result, "mock success");
            assert!(!resp.is_error);
        }
        _ => panic!("Second event must be Result"),
    }
}

#[tokio::test]
async fn test_send_prefers_transcript_over_stdout() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let temp_dir = tempfile::tempdir().unwrap();
    write_mock_script(temp_dir.path(), "#!/bin/sh\necho 'verbose narration on stdout'\n");
    
    let mut path_env = std::env::var("PATH").unwrap_or_default();
    path_env = format!("{}:{}", temp_dir.path().to_string_lossy(), path_env);
    unsafe {
        std::env::set_var("PATH", path_env);
        std::env::set_var("HOME", temp_dir.path().to_string_lossy().to_string());
    }

    let workspace = temp_dir.path().to_path_buf();
    let cache_dir = temp_dir.path().join(".gemini").join("antigravity-cli").join("cache");
    std::fs::create_dir_all(&cache_dir).unwrap();
    let mapping = serde_json::json!({
        workspace.to_string_lossy().to_string(): "conv-1"
    });
    std::fs::write(cache_dir.join("last_conversations.json"), mapping.to_string()).unwrap();

    let logs_dir = temp_dir.path().join(".gemini").join("antigravity-cli").join("brain").join("conv-1").join(".system_generated").join("logs");
    std::fs::create_dir_all(&logs_dir).unwrap();
    let transcript_entry = serde_json::json!({
        "source": "MODEL",
        "type": "PLANNER_RESPONSE",
        "status": "DONE",
        "content": "clean answer"
    });
    std::fs::write(logs_dir.join("transcript_full.jsonl"), transcript_entry.to_string() + "\n").unwrap();

    let config = CliConfig {
        provider: "antigravity".to_string(),
        working_dir: temp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let resp = cli.send("hi", Some("conv-1"), false, workspace).await.unwrap();

    assert_eq!(resp.result, "clean answer");
}

#[tokio::test]
async fn test_send_falls_back_to_stdout_without_transcript() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let temp_dir = tempfile::tempdir().unwrap();
    write_mock_script(temp_dir.path(), "#!/bin/sh\necho '{\"result\":\"plain stdout answer\"}'\n");
    
    let mut path_env = std::env::var("PATH").unwrap_or_default();
    path_env = format!("{}:{}", temp_dir.path().to_string_lossy(), path_env);
    unsafe {
        std::env::set_var("PATH", path_env);
        std::env::set_var("HOME", temp_dir.path().to_string_lossy().to_string());
    }

    let config = CliConfig {
        provider: "antigravity".to_string(),
        working_dir: temp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let workspace = temp_dir.path().to_path_buf();
    let resp = cli.send("hi", None, false, workspace).await.unwrap();

    assert_eq!(resp.result, "plain stdout answer");
}

#[tokio::test]
async fn test_send_automatically_trusts_workspace() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let temp_dir = tempfile::tempdir().unwrap();
    
    // Create the expected parent folder structure inside the mock HOME
    let parent = temp_dir.path().join(".gemini").join("antigravity-cli");
    std::fs::create_dir_all(&parent).unwrap();

    write_mock_script(temp_dir.path(), "#!/bin/sh\necho '{\"result\":\"ok\"}'\n");
    
    let mut path_env = std::env::var("PATH").unwrap_or_default();
    path_env = format!("{}:{}", temp_dir.path().to_string_lossy(), path_env);
    unsafe {
        std::env::set_var("PATH", path_env);
        std::env::set_var("HOME", temp_dir.path().to_string_lossy().to_string());
    }

    let config = CliConfig {
        provider: "antigravity".to_string(),
        working_dir: temp_dir.path().to_path_buf(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let workspace = temp_dir.path().to_path_buf();
    
    // Trigger send
    let _ = cli.send("hi", None, false, workspace.clone()).await.unwrap();

    // Verify settings.json was created and workspace path trusted
    let settings_path = parent.join("settings.json");
    assert!(settings_path.exists());

    let content = std::fs::read_to_string(settings_path).unwrap();
    let data: serde_json::Value = serde_json::from_str(&content).unwrap();
    let workspaces = data.get("trustedWorkspaces").unwrap().as_array().unwrap();
    
    let expected = workspace.canonicalize().unwrap().to_string_lossy().to_string();
    assert!(workspaces.iter().any(|v| v.as_str() == Some(&expected)));
}

#[test]
fn test_is_session_alive_detects_valid_and_expired_sessions() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let temp_dir = tempfile::tempdir().unwrap();
    let home = temp_dir.path();
    unsafe {
        std::env::set_var("HOME", home.to_string_lossy().to_string());
    }

    let config = CliConfig {
        provider: "antigravity".to_string(),
        working_dir: home.to_path_buf(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);

    // Empty session
    assert!(!cli.is_session_alive(""));

    // Non-existent session
    assert!(!cli.is_session_alive("dead-uuid-1234"));

    // Existing session via conversation db
    let conv_dir = home.join(".gemini").join("antigravity-cli").join("conversations");
    std::fs::create_dir_all(&conv_dir).unwrap();
    std::fs::write(conv_dir.join("live-uuid-1.db"), b"").unwrap();
    assert!(cli.is_session_alive("live-uuid-1"));

    // Existing session via brain transcript
    let brain_logs = home.join(".gemini").join("antigravity-cli").join("brain").join("live-uuid-2").join(".system_generated").join("logs");
    std::fs::create_dir_all(&brain_logs).unwrap();
    std::fs::write(brain_logs.join("transcript_full.jsonl"), b"test").unwrap();
    assert!(cli.is_session_alive("live-uuid-2"));
}

#[tokio::test]
async fn test_run_in_pty_session_fails_fast_on_expired_session() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let temp_dir = tempfile::tempdir().unwrap();
    let home = temp_dir.path();
    unsafe {
        std::env::set_var("HOME", home.to_string_lossy().to_string());
    }

    let config = CliConfig {
        provider: "antigravity".to_string(),
        working_dir: home.to_path_buf(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let start = std::time::Instant::now();
    let res = cli.send("hello", Some("expired-uuid-9999"), false, home.to_path_buf()).await;
    assert!(res.is_err());
    let err = res.err().unwrap();
    assert!(err.contains("expired or not found in Antigravity storage"));
    assert!(start.elapsed().as_secs() < 5);
}

