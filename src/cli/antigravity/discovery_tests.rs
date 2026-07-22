//! # Antigravity Model Discovery Tests
//!
//! This module validates the parsing of output from the `agy models` command
//! and the discovery execution behavior.

#[cfg(test)]
mod tests {
    use crate::cli::antigravity::discovery::{parse_models, discover_models};

    const SAMPLE_OUTPUT: &str = "gemini-3.6-flash-high\ngemini-3.1-pro-low\nclaude-sonnet-4-6\n";

    #[test]
    fn test_parse_models_keeps_display_names() {
        let parsed = parse_models(SAMPLE_OUTPUT);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0], "gemini-3.6-flash-high");
        assert_eq!(parsed[1], "gemini-3.1-pro-low");
        assert_eq!(parsed[2], "claude-sonnet-4-6");
    }

    #[test]
    fn test_parse_models_skips_blank_lines() {
        let parsed = parse_models("\ngemini-3.6-flash-high\n\n");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0], "gemini-3.6-flash-high");
    }

    #[test]
    fn test_parse_models_rejects_usage_banner() {
        let parsed = parse_models("Usage: agy models [flags]\n\nList available models");
        assert!(parsed.is_empty());
    }

    #[tokio::test]
    async fn test_discover_models_returns_empty_when_agy_missing() {
        // If we force an invalid agy command or clean PATH, it should return empty vector safely.
        // We can simulate agy command check or run discover_models directly.
        // If agy is installed on the host, it might discover something, but if it fails/missing, it returns empty.
        let models = discover_models("nonexistent_agy_command_xyz").await;
        assert!(models.is_empty());
    }
}
