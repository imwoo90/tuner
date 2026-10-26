//! # Antigravity CLI Driver (index.md)
//!
//! ## Overview
//! Controls the `agy` executable, parsing real-time transcript logs, managing pseudo-terminals (PTY),
//! and validating workspace security rules.
//!
//! ## Submodules
//! - [`events`] (`events.rs`): Brain directory resolution and JSONL transcript answer extraction.
//! - [`session`] (`session.rs`): Active PTY session registry, soft Esc interruption, and ask state.
//! - [`provider`] (`provider.rs`): High-level `AgentProvider` implementation for oneshot and PTY runs.
//! - [`pty_spawner`] (`pty_spawner.rs`): Low-level non-echoing PTY allocation and child process spawning.
//! - [`log_helpers`] (`log_helpers.rs`): Byte-offset file tailing and collapsible Markdown formatting helpers.
//! - [`log_parser`] (`log_parser.rs`): Incremental structured log delta parser for CoT and tool calls.
//! - [`polling`] (`polling.rs`): File-watcher-driven transcript polling and completion loops.
//! - [`completion`] (`completion.rs`): Turn completion detection and PTY output stability checks.
//! - [`trust`] (`trust.rs`): Automatic workspace trust registration and dynamic model discovery.
//! - [`error_parser`] (`error_parser.rs`): User-friendly classification of CLI quota and runtime errors.
//!
//! ## Search Tags
//! #antigravity-driver, #pty-spawner, #log-extractor, #workspace-trust

use crate::config::CliConfig;
use std::path::PathBuf;

pub mod events;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
/// Test module/item.
pub mod events_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
/// Test module/item.
pub mod async_tests;
pub mod session;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
/// Test module/item.
pub mod session_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod interrupt_tests;
pub mod provider;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
/// Test module/item.
pub mod provider_tests;
pub mod pty_spawner;
pub mod log_helpers;
pub mod log_parser;
pub mod polling;
pub mod completion;
/// Dynamic model discovery re-exports.
pub mod discovery {
    pub use super::trust::{discover_models, parse_models};
}
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod discovery_tests;
pub mod trust;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod trust_tests;
pub mod error_parser;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod error_parser_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod provider_json_tests;

#[derive(Clone)]
/// Represents `AntigravityCli` (antigravity cli).
pub struct AntigravityCli {
    /// Config.
    pub config: CliConfig,
    /// Sessions.
    pub sessions: std::sync::Arc<session::SessionManager>,
}

impl AntigravityCli {
    /// Creates a new instance.
    pub fn new(config: CliConfig) -> Self {
        Self {
            config,
            sessions: std::sync::Arc::new(session::SessionManager::new()),
        }
    }

    /// Executes `is_session_alive` (is session alive).
    pub fn is_session_alive(&self, session_id: &str) -> bool {
        if session_id.is_empty() {
            return false;
        }
        let env = self.build_env();
        let root = events::agy_state_root(Some(&env));
        let conv_db = root.join("conversations").join(format!("{}.db", session_id));
        let transcript = root.join("brain").join(session_id).join(".system_generated").join("logs").join("transcript_full.jsonl");
        conv_db.exists() || transcript.exists()
    }

    /// Executes `discover_models` (discover models).
    pub async fn discover_models(&self) -> Vec<String> {
        discovery::discover_models("agy").await
    }

    pub(crate) fn normalize_model_id(raw: &str) -> &str {
        match raw {
            "opus" | "claude-opus" | "opus-5-5" | "opus5.5" => "claude-opus-5-5",
            "sonnet" | "claude-sonnet" | "sonnet-5-5" | "sonnet5.5" => "claude-sonnet-5-5",
            "flash" | "gemini" | "gemini-2.5-flash" | "gemini-3-flash" | "gemini-3.0-flash" | "gemini-3.5-flash"
            | "gemini-3.6-flash" | "gemini-3.7-flash" | "gemini-3.8-flash" => "gemini-3.8-flash",
            "argon" | "gemini-argon" | "gemini-4-argon" | "pro" | "gemini-2.5-pro"
            | "gemini-3.0-pro" | "gemini-3.1-pro" => "gemini-4-argon",
            other => other,
        }
    }

    pub(crate) fn resolve_cli_model_effort(model: Option<&str>, effort: Option<&str>) -> (Option<String>, Option<String>) {
        let Some(raw_m) = model.map(str::trim).filter(|&s| !s.is_empty() && s != "antigravity-default") else {
            return (None, None);
        };
        let mut base = raw_m;
        let mut suffix_eff = None;
        for (suf, eff_val) in [("-high", "high"), ("-medium", "medium"), ("-low", "low")] {
            if let Some(stripped) = raw_m.strip_suffix(suf) {
                base = stripped;
                suffix_eff = Some(eff_val.to_string());
                break;
            }
        }
        let canonical = Self::normalize_model_id(base);
        let mut eff = effort
            .map(str::trim)
            .filter(|&e| !e.is_empty())
            .map(str::to_string)
            .or(suffix_eff);
        if eff.is_none()
            && (canonical.contains("claude-")
                || canonical.contains("opus")
                || canonical.contains("sonnet")
                || canonical.contains("pro")
                || canonical.contains("argon"))
        {
            eff = Some("high".to_string());
        }
        (Some(canonical.to_string()), eff)
    }

    /// Executes `build_command` (build command).
    pub fn build_command(
        &self,
        prompt: &str,
        resume_session: Option<&str>,
        continue_session: bool,
    ) -> Vec<String> {
        let mut cmd = vec!["agy".to_string()];
        cmd.push("--add-dir".to_string());
        cmd.push(self.agy_workspace().to_string_lossy().to_string());

        let (m, eff) = Self::resolve_cli_model_effort(self.config.model.as_deref(), self.config.effort.as_deref());
        if let Some(model) = m {
            cmd.push("--model".to_string());
            cmd.push(model);
        }
        if let Some(effort) = eff {
            cmd.push("--effort".to_string());
            cmd.push(effort);
        }

        if let Some(session_id) = resume_session {
            cmd.push("--conversation".to_string());
            cmd.push(session_id.to_string());
        } else if continue_session {
            cmd.push("--continue".to_string());
        }

        if self.config.permission_mode == "bypassPermissions" {
            cmd.push("--dangerously-skip-permissions".to_string());
        }

        if let Some(params) = self.config.cli_parameters.get("antigravity") {
            for param in params {
                cmd.push(param.clone());
            }
        }

        let has_print_timeout = cmd.iter().any(|arg| arg == "--print-timeout" || arg.starts_with("--print-timeout="));
        if !has_print_timeout
            && let Ok(timeout_val) = std::env::var("TUNER_PRINT_TIMEOUT") {
                cmd.push("--print-timeout".to_string());
                cmd.push(timeout_val);
            }

        if !cmd.contains(&"--output-format".to_string()) {
            cmd.push("--output-format".to_string());
            cmd.push("json".to_string());
        }

        let final_prompt = self.format_prompt(prompt);
        cmd.push("--print".to_string());
        cmd.push(final_prompt);

        cmd
    }

    /// Executes `resolve_oneshot_timeout` (resolve oneshot timeout).
    pub fn resolve_oneshot_timeout(cmd_args: &[String]) -> (tokio::time::Duration, u64) {
        let explicit = cmd_args.iter().enumerate().find_map(|(i, arg)| {
            if arg == "--print-timeout" {
                cmd_args.get(i + 1)?.trim_end_matches('s').trim().parse::<u64>().ok()
            } else if let Some(val) = arg.strip_prefix("--print-timeout=") {
                val.trim_end_matches('s').trim().parse::<u64>().ok()
            } else {
                None
            }
        });
        let env_val = std::env::var("TUNER_CLI_TIMEOUT_SECS")
            .or_else(|_| std::env::var("TUNER_PRINT_TIMEOUT"))
            .ok()
            .and_then(|s| s.trim_end_matches('s').trim().parse::<u64>().ok());
        const DEFAULT_TIMEOUT_SECS: u64 = 3600;
        let base = explicit.or(env_val).unwrap_or(DEFAULT_TIMEOUT_SECS);
        let effective = if base == 0 {
            86400
        } else if explicit.is_some() {
            base + 15
        } else {
            base
        };
        (tokio::time::Duration::from_secs(effective), base)
    }

fn append_section(target: &mut String, content: &str) {
    let trimmed = content.trim();
    if !trimmed.is_empty() {
        if !target.is_empty() {
            target.push_str("\n\n");
        }
        target.push_str(trimmed);
    }
}

    pub(crate) fn format_prompt(&self, prompt: &str) -> String {
        let mut final_prompt = String::new();
        if let Some(ref sp) = self.config.system_prompt {
            Self::append_section(&mut final_prompt, sp);
        }
        if let Some(ref asp) = self.config.append_system_prompt {
            Self::append_section(&mut final_prompt, asp);
        }
        Self::append_section(&mut final_prompt, prompt);
        final_prompt
    }

    /// Executes `agy_workspace` (agy workspace).
    pub fn agy_workspace(&self) -> PathBuf {
        let working_dir = &self.config.working_dir;
        let path_str = working_dir.to_string_lossy();
        if !path_str.contains("/.") {
            return working_dir.clone();
        }

        let components: Vec<_> = working_dir.components().collect();
        for (index, comp) in components.iter().enumerate() {
            if let std::path::Component::Normal(os_str) = comp {
                let segment = os_str.to_string_lossy();
                if segment.starts_with('.') && segment != "." && segment != ".." {
                    let mut dot_ancestor = PathBuf::new();
                    for c in &components[..=index] {
                        dot_ancestor.push(c);
                    }

                    let link_name = &segment[1..];
                    let mut link = dot_ancestor.clone();
                    link.set_file_name(link_name);

                    let mut remainder = PathBuf::new();
                    for c in &components[index + 1..] {
                        remainder.push(c);
                    }

                    if self.ensure_agy_link(&link, &dot_ancestor) {
                        return link.join(remainder);
                    }
                    return working_dir.clone();
                }
            }
        }
        working_dir.clone()
    }

    fn ensure_agy_link(&self, link: &std::path::Path, target: &std::path::Path) -> bool {
        if link.is_symlink() {
            if let Ok(resolved) = link.canonicalize()
                && let Ok(target_canon) = target.canonicalize()
                    && resolved == target_canon {
                        return true;
                    }
            let _ = std::fs::remove_file(link);
        } else if link.exists() {
            return false;
        }

        if let Some(parent) = link.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        #[cfg(unix)]
        {
            if std::os::unix::fs::symlink(target, link).is_ok() {
                return link.exists();
            }
        }
        false
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
/// Test module/item.
pub mod mod_tests;
