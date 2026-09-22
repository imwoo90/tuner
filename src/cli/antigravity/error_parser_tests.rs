//! # CLI Error Parser Tests
//!
//! This module contains tests for parsing CLI execution failures
//! and providing actionable troubleshooting suggestions.

#[cfg(test)]
mod tests {
    use crate::cli::antigravity::error_parser::{extract_agy_error, parse_cli_error};

    #[test]
    fn test_parse_cli_error_missing_api_key() {
        let stderr = "Error: Anthropic API key not found. Please set ANTHROPIC_API_KEY environment variable.";
        let suggestion = parse_cli_error(stderr, 1);
        assert!(suggestion.contains("API key is missing"));
        assert!(suggestion.contains("~/.tuner/.env"));
    }

    #[test]
    fn test_parse_cli_error_permission_denied() {
        let stderr = "sh: 1: agy: Permission denied";
        let suggestion = parse_cli_error(stderr, 126);
        assert!(suggestion.contains("Permission denied"));
        assert!(suggestion.contains("chmod +x"));
    }

    #[test]
    fn test_parse_cli_error_command_not_found() {
        let stderr = "sh: 1: agy: not found";
        let suggestion = parse_cli_error(stderr, 127);
        assert!(suggestion.contains("agy CLI is not installed"));
        assert!(suggestion.contains("PATH"));
    }

    #[test]
    fn test_parse_cli_error_unknown() {
        let stderr = "Some weird unexpected compilation failure";
        let suggestion = parse_cli_error(stderr, 1);
        assert!(suggestion.contains("Some weird unexpected compilation failure"));
    }

    #[test]
    fn test_parse_cli_error_agy_error_rate_limit_retryable() {
        let stderr = r#"AGY_ERROR: {"status": "RESOURCE_EXHAUSTED", "code": 429, "message": "Resource has been exhausted (e.g. check quota).", "retryable": true, "error_id": "req-1234"}"#;
        let suggestion = parse_cli_error(stderr, 3);
        assert!(suggestion.contains("rate limit or quota exceeded"));
        assert!(suggestion.contains("wait a short while"));
        assert!(suggestion.contains("Details: Resource has been exhausted"));
        assert!(suggestion.contains("Error ID: req-1234"));
        assert!(!suggestion.contains("\"status\""));
    }

    #[test]
    fn test_parse_cli_error_agy_error_quota_non_retryable() {
        let stderr = r#"AGY_ERROR: {"status": "RESOURCE_EXHAUSTED", "code": 429, "message": "You have exceeded your monthly token quota.", "retryable": false, "error_id": "req-quota-99"}"#;
        let suggestion = parse_cli_error(stderr, 3);
        assert!(suggestion.contains("rate limit or quota exceeded"));
        assert!(suggestion.contains("quota has been exhausted"));
        assert!(suggestion.contains("/model"));
        assert!(suggestion.contains("Error ID: req-quota-99"));
    }

    #[test]
    fn test_parse_cli_error_agy_error_context_length() {
        let stderr = r#"AGY_ERROR: {"status": "INVALID_ARGUMENT", "code": 400, "message": "Request payload size exceeds maximum limit: context window exceeded 1048576 tokens.", "retryable": false, "error_id": "ctx-err"}"#;
        let suggestion = parse_cli_error(stderr, 3);
        assert!(suggestion.contains("Context window or token limit exceeded"));
        assert!(suggestion.contains("/new"));
        assert!(suggestion.contains("Details: Request payload size exceeds maximum limit"));
        assert!(suggestion.contains("Error ID: ctx-err"));
    }

    #[test]
    fn test_parse_cli_error_agy_error_unavailable_503() {
        let stderr = r#"AGY_ERROR: {"status": "UNAVAILABLE", "code": 503, "message": "The model is currently overloaded with other requests.", "retryable": true, "error_id": "err-503"}"#;
        let suggestion = parse_cli_error(stderr, 3);
        assert!(suggestion.contains("temporarily overloaded or unavailable"));
        assert!(suggestion.contains("/model"));
        assert!(suggestion.contains("Details: The model is currently overloaded"));
        assert!(suggestion.contains("Error ID: err-503"));
    }

    #[test]
    fn test_parse_cli_error_agy_error_model_not_found() {
        let stderr = r#"AGY_ERROR: {"status": "NOT_FOUND", "code": 404, "message": "Model 'claude-unknown-model' does not exist.", "retryable": false}"#;
        let suggestion = parse_cli_error(stderr, 3);
        assert!(suggestion.contains("Model not found or unsupported"));
        assert!(suggestion.contains("/model"));
        assert!(suggestion.contains("Details: Model 'claude-unknown-model' does not exist."));
    }

    #[test]
    fn test_parse_cli_error_agy_error_permission_denied() {
        let stderr = r#"AGY_ERROR: {"status": "PERMISSION_DENIED", "code": 403, "message": "API key lacks permission for the requested resource.", "retryable": false}"#;
        let suggestion = parse_cli_error(stderr, 3);
        assert!(suggestion.contains("Model API authentication or permission error"));
        assert!(suggestion.contains("~/.tuner/.env"));
        assert!(!suggestion.contains("chmod +x"));
    }

    #[test]
    fn test_parse_cli_error_agy_error_with_surrounding_noise() {
        let stderr = "2026-09-21 10:00:00 [INFO] agy headless running\n\
                      AGY_ERROR: {\"status\": \"RESOURCE_EXHAUSTED\", \"code\": 429, \"message\": \"Rate limit exceeded\", \"retryable\": true}\n\
                      2026-09-21 10:00:01 [WARN] Clean exit after failure\n";
        let suggestion = parse_cli_error(stderr, 3);
        assert!(suggestion.contains("rate limit or quota exceeded"));
        assert!(suggestion.contains("Details: Rate limit exceeded"));
    }

    #[test]
    fn test_parse_cli_error_exit_code_3_without_json() {
        let stderr = "Fatal agent invocation failure during turn initialization";
        let suggestion = parse_cli_error(stderr, 3);
        assert!(suggestion.contains("Antigravity model or agent failure (exit code 3)"));
        assert!(suggestion.contains("/model"));
        assert!(suggestion.contains("Details: Fatal agent invocation failure during turn initialization"));
    }

    #[test]
    fn test_parse_cli_error_command_timed_out() {
        let stderr = "Command timed out after 3600 seconds";
        let suggestion = parse_cli_error(stderr, 1);
        assert!(suggestion.contains("execution timed out"));
        assert!(suggestion.contains("smaller prompt"));
    }

    #[test]
    fn test_extract_agy_error_direct_json_fallback() {
        let stderr = "{\"status\": \"RESOURCE_EXHAUSTED\", \"code\": 429, \"message\": \"Quota exceeded\"}";
        let parsed = extract_agy_error(stderr);
        assert!(parsed.is_some());
        let payload = parsed.unwrap();
        assert_eq!(payload.status.as_deref(), Some("RESOURCE_EXHAUSTED"));
        assert_eq!(payload.code_as_i64(), Some(429));
    }

    #[test]
    fn test_parse_cli_error_nested_error_object() {
        let stderr = r#"AGY_ERROR: {"error": {"code": 429, "message": "Too many requests to this endpoint", "status": "RESOURCE_EXHAUSTED"}, "retryable": true}"#;
        let suggestion = parse_cli_error(stderr, 3);
        assert!(suggestion.contains("rate limit or quota exceeded"));
        assert!(suggestion.contains("wait a short while"));
        assert!(suggestion.contains("Details: Too many requests to this endpoint"));
    }

    #[test]
    fn test_parse_cli_error_string_code() {
        let stderr = r#"AGY_ERROR: {"code": "RESOURCE_EXHAUSTED", "message": "Daily usage limit reached", "retryable": false}"#;
        let suggestion = parse_cli_error(stderr, 3);
        assert!(suggestion.contains("rate limit or quota exceeded"));
        assert!(suggestion.contains("quota has been exhausted"));
        assert!(suggestion.contains("Details: Daily usage limit reached"));
    }

    #[test]
    fn test_parse_cli_error_context_prompt_too_long() {
        let stderr = r#"AGY_ERROR: {"status": "INVALID_ARGUMENT", "code": 400, "message": "The prompt is too long for this model.", "retryable": false}"#;
        let suggestion = parse_cli_error(stderr, 3);
        assert!(suggestion.contains("Context window or token limit exceeded"));
        assert!(suggestion.contains("/new"));
    }

    #[test]
    fn test_extract_agy_error_ignores_empty_json() {
        let stderr = "AGY_ERROR: {}\nSome normal stderr output";
        let parsed = extract_agy_error(stderr);
        assert!(parsed.is_none());
    }

    #[test]
    fn test_parse_cli_error_exit_code_3_in_message_text() {
        let stderr = "Process exited with error status: exit code 3";
        let suggestion = parse_cli_error(stderr, 1);
        assert!(suggestion.contains("exit code 3"));
        assert!(suggestion.contains("/model"));
    }

    #[test]
    fn test_parse_cli_error_unix_wait_status_768() {
        let stderr = "Process exited with error status: ExitStatus(unix_wait_status(768))";
        let suggestion = parse_cli_error(stderr, 1);
        assert!(suggestion.contains("exit code 3"));
        assert!(suggestion.contains("/model"));
    }
}

