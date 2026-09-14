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
