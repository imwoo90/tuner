//! # Smart CLI Error Parser
//!
//! This module analyzes agy CLI process exit status and stderr streams
//! to offer clear, actionable restoration guidelines to the user.

/// Analyze the process exit code and stderr to generate user-friendly troubleshooting suggestions.
pub fn parse_cli_error(stderr: &str, returncode: i32) -> String {
    let lower_stderr = stderr.to_lowercase();

    if lower_stderr.contains("api key not found") || lower_stderr.contains("api_key not found") || lower_stderr.contains("api key is missing") {
        return format!(
            "❌ [tuner] API key is missing or not configured.\n\
             💡 Suggestion: Write your API key into ~/.tuner/.env (e.g. ANTHROPIC_API_KEY=sk-...) to make it globally available."
        );
    }

    if returncode == 126 || lower_stderr.contains("permission denied") {
        return format!(
            "❌ [tuner] Permission denied when executing agy.\n\
             💡 Suggestion: Ensure that the 'agy' binary is executable (e.g. run 'chmod +x <path-to-agy>')."
        );
    }

    if returncode == 127
        || lower_stderr.contains("command not found")
        || lower_stderr.contains("agy: not found")
        || lower_stderr.contains("no such file or directory")
    {
        return format!(
            "❌ [tuner] agy CLI is not installed or not available in the system PATH.\n\
             💡 Suggestion: Verify if 'agy' is installed and that its location is appended to your PATH environment variable."
        );
    }

    stderr.to_string()
}
