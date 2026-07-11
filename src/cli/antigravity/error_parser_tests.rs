//! # CLI Error Parser Tests
//!
//! This module contains tests for parsing CLI execution failures
//! and providing actionable troubleshooting suggestions.

#[cfg(test)]
mod tests {
    use crate::cli::antigravity::error_parser::parse_cli_error;

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
}
