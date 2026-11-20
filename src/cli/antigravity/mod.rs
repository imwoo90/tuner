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
//! - [`log_watcher`] (`log_watcher.rs`): Single-directory non-recursive transcript filesystem watchers.
//! - [`polling`] (`polling.rs`): File-watcher-driven transcript polling and completion loops.
//! - [`completion`] (`completion.rs`): Turn completion detection and PTY output stability checks.
//! - [`trust`] (`trust.rs`): Automatic workspace trust registration and dynamic model discovery.
//! - [`model_spec`] (`model_spec.rs`): Model alias normalization, effortless orchestrator rules, and `--effort` clamping.
//! - [`pty_sync`] (`pty_sync.rs`): Per-session PTY prompt initialization lock, applied `/model` tracking, and turn guards.
//! - [`pty_model_sync`] (`pty_model_sync.rs`): Warm PTY `/model` injection and background-descendant-safe default resets.
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
/// Non-recursive transcript directory watchers.
pub mod log_watcher;
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
/// Antigravity model alias normalization and reasoning effort compatibility rules.
pub mod model_spec;
/// Per-session PTY prompt readiness lock, applied `/model` tracking, and topic turn guards.
pub mod pty_sync;
/// Warm PTY `/model` target synchronization and background-descendant-safe default resets.
pub mod pty_model_sync;
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
    /// Marks stateless one-shot or cron CLI handles that run outside foreground warm pools.
    pub oneshot: bool,
    pub(crate) pty_sync: std::sync::Arc<pty_sync::PtySyncState>,
}

impl AntigravityCli {
    /// Creates a new instance.
    pub fn new(config: CliConfig) -> Self {
        Self {
            config,
            sessions: std::sync::Arc::new(session::SessionManager::new()),
            oneshot: false,
            pty_sync: std::sync::Arc::new(pty_sync::PtySyncState::default()),
        }
    }

    /// Creates a stateless one-shot CLI handle that runs outside foreground warm session pools.
    pub fn new_oneshot(config: CliConfig) -> Self {
        let mut cli = Self::new(config);
        cli.oneshot = true;
        cli
    }

    pub(crate) fn canonical_provider(&self) -> &'static str {
        crate::session::data::SessionData::effective_provider(&self.config.provider, "antigravity")
    }

    /// Executes `is_session_alive` (is session alive).
    pub fn is_session_alive(&self, session_id: &str) -> bool {
        if session_id.is_empty() {
            return false;
        }
        let env = self.build_env();
        let prov = self.canonical_provider();
        if matches!(prov, "claude" | "codex") {
            return crate::cli::common::is_peer_session_alive(prov, session_id, &env);
        }
        let root = events::agy_state_root(Some(&env));
        let conv_db = root.join("conversations").join(format!("{}.db", session_id));
        let transcript = root.join("brain").join(session_id).join(".system_generated").join("logs").join("transcript_full.jsonl");
        conv_db.exists() || transcript.exists()
    }

    pub(crate) fn build_env(&self) -> std::collections::HashMap<String, String> {
        let mut env: std::collections::HashMap<_, _> = std::env::vars().collect();
        let th = std::env::var("TUNER_HOME").ok().map(PathBuf::from).unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".tuner")
        });
        env.extend([
            ("TUNER_AGENT_NAME".into(), "main".into()),
            ("TUNER_CHAT_ID".into(), self.config.chat_id.to_string()),
            ("TUNER_TRANSPORT".into(), self.config.transport.clone()),
            ("TERM".into(), "dumb".into()),
            ("NO_COLOR".into(), "1".into()),
            ("SSH_CONNECTION".into(), "127.0.0.1 0 127.0.0.1 22".into()),
            ("TUNER_HOME".into(), th.to_string_lossy().into()),
        ]);
        if let Some(tid) = self.config.topic_id {
            env.insert("TUNER_TOPIC_ID".into(), tid.to_string());
        }
        env
    }

    /// Executes `discover_models` (discover models).
    pub async fn discover_models(&self) -> Vec<String> {
        let env = self.build_env();
        match self.canonical_provider() {
            "claude" => crate::cli::claude::discover_models_live(&env).await,
            "codex" => crate::cli::codex::discover_models_live(&env).await,
            _ => {
                let models = discovery::discover_models("agy").await;
                model_spec::update_live_catalog(&models);
                models
            }
        }
    }

    /// Returns `true` when the given Antigravity model accepts `--effort` / `-<effort>` levels.
    pub fn supports_effort(model: &str) -> bool {
        model_spec::supports_effort(model)
    }

    /// Returns the ordered reasoning effort levels supported by the given Antigravity model.
    pub fn supported_efforts_for_model(model: &str) -> Vec<String> {
        model_spec::supported_efforts(model)
    }

    pub(crate) fn resolve_cli_model_effort(
        model: Option<&str>,
        effort: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        model_spec::resolve_cli_model_effort(model, effort)
    }

    /// Executes `build_command` (build command).
    pub fn build_command(
        &self,
        prompt: &str,
        resume_session: Option<&str>,
        continue_session: bool,
    ) -> Vec<String> {
        match self.canonical_provider() {
            "claude" => return crate::cli::claude::build_command(self, prompt, resume_session, continue_session),
            "codex" => return crate::cli::codex::build_command(self, prompt, resume_session, continue_session),
            _ => {}
        }
        let mut cmd = vec![
            "agy".to_string(),
            "--add-dir".to_string(),
            self.agy_workspace().to_string_lossy().to_string(),
        ];
        let (m, eff) = Self::resolve_cli_model_effort(self.config.model.as_deref(), self.config.effort.as_deref());
        if let Some(model) = m { cmd.extend(["--model".to_string(), model]); }
        if let Some(effort) = eff { cmd.extend(["--effort".to_string(), effort]); }
        if let Some(sid) = resume_session {
            cmd.extend(["--conversation".to_string(), sid.to_string()]);
        } else if continue_session {
            cmd.push("--continue".to_string());
        }
        if self.config.permission_mode == "bypassPermissions" {
            cmd.push("--dangerously-skip-permissions".to_string());
        }
        if let Some(params) = self.config.cli_parameters.get("antigravity") {
            cmd.extend(params.iter().cloned());
        }
        if !cmd.iter().any(|a| a == "--print-timeout" || a.starts_with("--print-timeout="))
            && let Ok(val) = std::env::var("TUNER_PRINT_TIMEOUT")
        {
            cmd.extend(["--print-timeout".to_string(), val]);
        }
        if !cmd.contains(&"--output-format".to_string()) {
            cmd.extend(["--output-format".to_string(), "json".to_string()]);
        }
        cmd.extend(["--print".to_string(), self.format_prompt(prompt)]);
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
            if let (Ok(r), Ok(tc)) = (link.canonicalize(), target.canonicalize()) && r == tc {
                return true;
            }
            let _ = std::fs::remove_file(link);
        } else if link.exists() {
            return false;
        }
        if let Some(parent) = link.parent() { let _ = std::fs::create_dir_all(parent); }
        #[cfg(unix)]
        if std::os::unix::fs::symlink(target, link).is_ok() { return link.exists(); }
        false
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
/// Test module/item.
pub mod mod_tests;
