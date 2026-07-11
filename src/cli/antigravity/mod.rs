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

    fn format_prompt(&self, prompt: &str) -> String {
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
mod tests {
    use super::*;
    use crate::config::CliConfig;

    #[test]
    fn test_antigravity_command_uses_print_and_conversation() {
        let config = CliConfig {
            provider: "antigravity".to_string(),
            model: Some("antigravity-default".to_string()),
            ..Default::default()
        };
        let cli = AntigravityCli::new(config);
        let cmd = cli.build_command("hi there", Some("conv-1"), false);

        assert_eq!(cmd[0], "agy");
        assert_eq!(cmd.iter().filter(|&&ref s| s == "--model").count(), 0);
        assert!(cmd.contains(&"--conversation".to_string()));
        assert!(cmd.contains(&"conv-1".to_string()));
        assert_eq!(cmd[cmd.len() - 2..], ["--print".to_string(), "hi there".to_string()]);
    }

    #[test]
    fn test_antigravity_command_grounds_in_workspace() {
        let config = CliConfig {
            provider: "antigravity".to_string(),
            working_dir: PathBuf::from("."),
            ..Default::default()
        };
        let cli = AntigravityCli::new(config);
        let cmd = cli.build_command("hi", None, false);

        assert!(cmd.contains(&"--add-dir".to_string()));
        let idx = cmd.iter().position(|s| s == "--add-dir").unwrap();
        assert_eq!(cmd[idx + 1], cli.agy_workspace().to_string_lossy().to_string());
    }

    #[test]
    fn test_antigravity_command_includes_selected_model() {
        let config = CliConfig {
            provider: "antigravity".to_string(),
            model: Some("claude-sonnet-4-5".to_string()),
            ..Default::default()
        };
        let cli = AntigravityCli::new(config);
        let cmd = cli.build_command("hi", None, false);

        assert!(cmd.contains(&"--model".to_string()));
        let model_idx = cmd.iter().position(|s| s == "--model").unwrap();
        assert_eq!(cmd[model_idx + 1], "claude-sonnet-4-5");
        
        let print_idx = cmd.iter().position(|s| s == "--print").unwrap();
        assert!(model_idx < print_idx);
    }

    #[test]
    fn test_antigravity_command_continue_and_bypass() {
        let config = CliConfig {
            provider: "antigravity".to_string(),
            permission_mode: "bypassPermissions".to_string(),
            ..Default::default()
        };
        let cli = AntigravityCli::new(config);
        let cmd = cli.build_command("hi", None, true);

        assert!(cmd.contains(&"--continue".to_string()));
        assert!(cmd.contains(&"--dangerously-skip-permissions".to_string()));
        assert_eq!(cmd[cmd.len() - 2..], ["--print".to_string(), "hi".to_string()]);
    }

    #[test]
    fn test_antigravity_command_includes_cli_parameters() {
        let mut cli_params = std::collections::HashMap::new();
        cli_params.insert("antigravity".to_string(), vec!["--log-file".to_string(), "agy.log".to_string()]);
        let config = CliConfig {
            provider: "antigravity".to_string(),
            cli_parameters: cli_params,
            ..Default::default()
        };
        let cli = AntigravityCli::new(config);
        let cmd = cli.build_command("hi", None, false);

        let log_idx = cmd.iter().position(|s| s == "--log-file").unwrap();
        assert_eq!(cmd[log_idx + 1], "agy.log");
        
        let print_idx = cmd.iter().position(|s| s == "--print").unwrap();
        assert!(log_idx < print_idx);
        assert_eq!(cmd[cmd.len() - 2..], ["--print".to_string(), "hi".to_string()]);
    }

    fn create_test_dir(name: &str) -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("target");
        p.push("test_dirs");
        p.push(name);
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn test_antigravity_dotted_workspace_mapped_to_symlink() {
        let base = create_test_dir("dotted_test");
        // Since `base` is located under `/home/wimvm/.ductor/`, it already has a dot-prefixed ancestor.
        // It will be mapped to `/home/wimvm/ductor/...` which has no dots.

        let config = CliConfig {
            provider: "antigravity".to_string(),
            working_dir: base.clone(),
            ..Default::default()
        };
        let cli = AntigravityCli::new(config);
        let agy_ws = cli.agy_workspace();

        let agy_ws_str = agy_ws.to_string_lossy();
        assert!(!agy_ws_str.contains("/."));
        assert_eq!(agy_ws.canonicalize().unwrap(), base.canonicalize().unwrap());

        let cmd = cli.build_command("hi", None, false);
        let idx = cmd.iter().position(|s| s == "--add-dir").unwrap();
        assert_eq!(cmd[idx + 1], agy_ws.to_string_lossy().to_string());
    }

    #[test]
    fn test_antigravity_plain_workspace_unchanged() {
        // Use a fictional path that has no dot-prefixed ancestors.
        // This avoids filesystem operations and returns the path unchanged.
        let plain = PathBuf::from("/home/wimvm/projects/plain_test");

        let config = CliConfig {
            provider: "antigravity".to_string(),
            working_dir: plain.clone(),
            ..Default::default()
        };
        let cli = AntigravityCli::new(config);
        
        assert_eq!(cli.agy_workspace(), plain);
    }
}
