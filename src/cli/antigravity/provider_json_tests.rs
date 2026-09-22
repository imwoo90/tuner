//! # Provider JSON Output and Session ID Extraction Tests
//!
//! Validates that `extract_conversation_id` accurately parses JSON output emitted by
//! Antigravity CLI's `--output-format json`, correctly handling clean JSON, multiline logs,
//! and warning banners to prevent concurrent topic session ID collision.

use super::*;
use crate::config::CliConfig;

#[test]
fn test_extract_conversation_id_clean_json() {
    let raw = r#"{"conversation_id":"831ff15f-677c-4290-bf43-3fc86bd7a9de","status":"SUCCESS","response":"Hello! How can I help you today?\n","duration_seconds":1.88}"#;
    let extracted = events::extract_conversation_id(raw);
    assert_eq!(extracted, Some("831ff15f-677c-4290-bf43-3fc86bd7a9de".to_string()));
}

#[test]
fn test_extract_conversation_id_with_warning_and_noise() {
    let raw = "warning: some debug message\n{\"conversation_id\":\"unique-session-456\",\"status\":\"SUCCESS\"}\nclosing message\n";
    let extracted = events::extract_conversation_id(raw);
    assert_eq!(extracted, Some("unique-session-456".to_string()));
}

#[test]
fn test_extract_conversation_id_empty_or_invalid() {
    assert_eq!(events::extract_conversation_id(""), None);
    assert_eq!(events::extract_conversation_id("plain text without json"), None);
    assert_eq!(events::extract_conversation_id("{\"status\":\"SUCCESS\"}"), None);
    assert_eq!(events::extract_conversation_id("{\"conversation_id\":\"\"}"), None);
}

#[test]
fn test_parse_antigravity_json_extracts_response_field() {
    let raw = r#"{"conversation_id":"abc-123","status":"SUCCESS","response":"Hello, I am Tuner!"}"#;
    let parsed = events::parse_antigravity_json(raw);
    assert_eq!(parsed, "Hello, I am Tuner!");
}

#[test]
fn test_build_command_includes_output_format_json() {
    let config = CliConfig {
        provider: "antigravity".to_string(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let cmd = cli.build_command("hello test", None, false);

    assert!(cmd.contains(&"--output-format".to_string()));
    let idx = cmd.iter().position(|s| s == "--output-format").unwrap();
    assert_eq!(cmd[idx + 1], "json");

    let print_idx = cmd.iter().position(|s| s == "--print").unwrap();
    assert!(idx < print_idx);
}

static TIMEOUT_TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn test_resolve_oneshot_timeout_default() {
    let _guard = TIMEOUT_TEST_MUTEX.lock().unwrap();
    let args: Vec<String> = vec!["agy".into(), "--print".into(), "hello".into()];
    let (duration, display) = AntigravityCli::resolve_oneshot_timeout(&args);
    assert_eq!(display, 3600);
    assert_eq!(duration, std::time::Duration::from_secs(3600));
}

#[test]
fn test_resolve_oneshot_timeout_from_cmd_args() {
    let args1: Vec<String> = vec!["agy".into(), "--print-timeout".into(), "600".into()];
    let (duration1, display1) = AntigravityCli::resolve_oneshot_timeout(&args1);
    assert_eq!(display1, 600);
    assert_eq!(duration1, std::time::Duration::from_secs(615));

    let args2: Vec<String> = vec!["agy".into(), "--print-timeout".into(), "1200s".into()];
    let (duration2, display2) = AntigravityCli::resolve_oneshot_timeout(&args2);
    assert_eq!(display2, 1200);
    assert_eq!(duration2, std::time::Duration::from_secs(1215));
}

#[test]
fn test_resolve_oneshot_timeout_unlimited() {
    let args: Vec<String> = vec!["agy".into(), "--print-timeout".into(), "0".into()];
    let (duration, display) = AntigravityCli::resolve_oneshot_timeout(&args);
    assert_eq!(display, 0);
    assert_eq!(duration, std::time::Duration::from_secs(86400));
}

#[test]
fn test_resolve_oneshot_timeout_from_env() {
    let _guard = TIMEOUT_TEST_MUTEX.lock().unwrap();
    unsafe {
        std::env::set_var("TUNER_CLI_TIMEOUT_SECS", "1800");
    }
    let args: Vec<String> = vec!["agy".into(), "--print".into(), "hi".into()];
    let (duration, display) = AntigravityCli::resolve_oneshot_timeout(&args);
    assert_eq!(display, 1800);
    assert_eq!(duration, std::time::Duration::from_secs(1800));
    unsafe {
        std::env::remove_var("TUNER_CLI_TIMEOUT_SECS");
    }
}

#[test]
fn test_build_command_includes_print_timeout_from_env() {
    let _guard = TIMEOUT_TEST_MUTEX.lock().unwrap();
    unsafe {
        std::env::set_var("TUNER_PRINT_TIMEOUT", "1800s");
    }
    let config = CliConfig {
        provider: "antigravity".to_string(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let cmd = cli.build_command("hello test", None, false);

    assert!(cmd.contains(&"--print-timeout".to_string()));
    let idx = cmd.iter().position(|s| s == "--print-timeout").unwrap();
    assert_eq!(cmd[idx + 1], "1800s");
    let print_idx = cmd.iter().position(|s| s == "--print").unwrap();
    assert!(idx < print_idx);
    unsafe {
        std::env::remove_var("TUNER_PRINT_TIMEOUT");
    }
}

#[test]
fn test_build_command_preserves_cli_parameters_print_timeout() {
    let _guard = TIMEOUT_TEST_MUTEX.lock().unwrap();
    unsafe {
        std::env::set_var("TUNER_PRINT_TIMEOUT", "1800s");
    }
    let mut cli_params = std::collections::HashMap::new();
    cli_params.insert(
        "antigravity".to_string(),
        vec!["--print-timeout".to_string(), "600s".to_string()],
    );
    let config = CliConfig {
        provider: "antigravity".to_string(),
        cli_parameters: cli_params,
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let cmd = cli.build_command("hello test", None, false);

    let count = cmd.iter().filter(|&s| s == "--print-timeout").count();
    assert_eq!(count, 1);
    let idx = cmd.iter().position(|s| s == "--print-timeout").unwrap();
    assert_eq!(cmd[idx + 1], "600s");
    unsafe {
        std::env::remove_var("TUNER_PRINT_TIMEOUT");
    }
}

#[test]
fn test_resolve_oneshot_timeout_from_cmd_args_equals_syntax() {
    let args1: Vec<String> = vec!["agy".into(), "--print-timeout=600".into()];
    let (duration1, display1) = AntigravityCli::resolve_oneshot_timeout(&args1);
    assert_eq!(display1, 600);
    assert_eq!(duration1, std::time::Duration::from_secs(615));

    let args2: Vec<String> = vec!["agy".into(), "--print-timeout=1200s".into()];
    let (duration2, display2) = AntigravityCli::resolve_oneshot_timeout(&args2);
    assert_eq!(display2, 1200);
    assert_eq!(duration2, std::time::Duration::from_secs(1215));
}

#[test]
fn test_build_command_preserves_cli_parameters_print_timeout_equals_syntax() {
    let _guard = TIMEOUT_TEST_MUTEX.lock().unwrap();
    unsafe {
        std::env::set_var("TUNER_PRINT_TIMEOUT", "1800s");
    }
    let mut cli_params = std::collections::HashMap::new();
    cli_params.insert(
        "antigravity".to_string(),
        vec!["--print-timeout=600s".to_string()],
    );
    let config = CliConfig {
        provider: "antigravity".to_string(),
        cli_parameters: cli_params,
        ..Default::default()
    };
    let cli = AntigravityCli::new(config);
    let cmd = cli.build_command("hello test", None, false);

    assert!(cmd.contains(&"--print-timeout=600s".to_string()));
    assert!(!cmd.contains(&"--print-timeout".to_string()));
    unsafe {
        std::env::remove_var("TUNER_PRINT_TIMEOUT");
    }
}

