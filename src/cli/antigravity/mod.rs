//! # Antigravity CLI Provider
//!
//! This module implements the provider for controlling and interacting with the Google Antigravity CLI (`agy`).
//! It supports the dotted-workspace bypass mechanism ([`AntigravityCli::agy_workspace`]) and the parameter builder ([`AntigravityCli::build_command`]).

use crate::config::CliConfig;
use std::path::PathBuf;

pub mod events;
#[cfg(test)]
pub mod events_tests;
pub mod session;
#[cfg(test)]
pub mod session_tests;
pub mod provider;
#[cfg(test)]
pub mod provider_tests;
pub mod log_parser;
pub mod discovery;
#[cfg(test)]
pub mod discovery_tests;
pub mod trust;
#[cfg(test)]
pub mod trust_tests;
pub mod error_parser;
#[cfg(test)]
pub mod error_parser_tests;

#[derive(Clone)]
pub struct AntigravityCli {
    pub config: CliConfig,
    pub sessions: std::sync::Arc<session::SessionManager>,
}

impl AntigravityCli {
    pub fn new(config: CliConfig) -> Self {
        Self {
            config,
            sessions: std::sync::Arc::new(session::SessionManager::new()),
        }
    }

    pub async fn discover_models(&self) -> Vec<String> {
        discovery::discover_models("agy").await
    }

    pub fn build_command(
        &self,
        prompt: &str,
        resume_session: Option<&str>,
        continue_session: bool,
    ) -> Vec<String> {
        let mut cmd = vec!["agy".to_string()];

        cmd.push("--add-dir".to_string());
        cmd.push(self.agy_workspace().to_string_lossy().to_string());

        if let Some(ref model) = self.config.model {
            if model != "antigravity-default" {
                cmd.push("--model".to_string());
                cmd.push(model.clone());
            }
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

        let final_prompt = self.format_prompt(prompt);
        cmd.push("--print".to_string());
        cmd.push(final_prompt);

        cmd
    }

    pub(crate) fn format_prompt(&self, prompt: &str) -> String {
        let mut final_prompt = String::new();
        if let Some(ref sp) = self.config.system_prompt {
            final_prompt.push_str(sp);
        }
        if let Some(ref asp) = self.config.append_system_prompt {
            if !final_prompt.is_empty() {
                final_prompt.push_str("\n\n");
            }
            final_prompt.push_str(asp);
        }

        let rules_path = self.config.working_dir.join("GEMINI.md");
        if let Ok(rules) = std::fs::read_to_string(rules_path) {
            let rules_trimmed = rules.trim();
            if !rules_trimmed.is_empty() {
                if !final_prompt.is_empty() {
                    final_prompt.push_str("\n\n");
                }
                final_prompt.push_str(rules_trimmed);
            }
        }

        let mem_path = self.config.working_dir.join("memory_system").join("MAINMEMORY.md");
        if let Ok(mem) = std::fs::read_to_string(mem_path) {
            let mem_trimmed = mem.trim();
            if !mem_trimmed.is_empty() {
                if !final_prompt.is_empty() {
                    final_prompt.push_str("\n\n");
                }
                final_prompt.push_str(mem_trimmed);
            }
        }

        if !final_prompt.is_empty() {
            final_prompt.push_str("\n\n");
        }
        final_prompt.push_str(prompt);
        final_prompt
    }

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
            if let Ok(resolved) = link.canonicalize() {
                if let Ok(target_canon) = target.canonicalize() {
                    if resolved == target_canon {
                        return true;
                    }
                }
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
pub mod mod_tests;
