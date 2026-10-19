//! # Antigravity CLI Provider
//! Implements the AgentProvider trait for AntigravityCli, managing execution environments, PTY lifecycles, and logs.

use crate::cli::antigravity::AntigravityCli;
use crate::cli::{AgentProvider, CliResponse, StreamEvent};
use crate::cli::antigravity::events;
use async_trait::async_trait;
use futures::stream::BoxStream;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
#[allow(unused_imports)]
pub(crate) use super::pty_spawner::strip_ansi;

impl AntigravityCli {
    pub(crate) fn build_env(&self) -> HashMap<String, String> {
        let mut env: HashMap<_, _> = std::env::vars().filter(|(k, _)| k != "CODEX_SANDBOX_NETWORK_DISABLED").collect();
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
            ("TUNER_SHARED_MEMORY_PATH".into(), th.join("SHAREDMEMORY.md").to_string_lossy().into()),
            ("TUNER_HOME".into(), th.to_string_lossy().into()),
        ]);
        if let Some(tid) = self.config.topic_id { env.insert("TUNER_TOPIC_ID".into(), tid.to_string()); }
        env
    }

    async fn ensure_interactive_session(&self, session_id: &str, _workspace: &Path, env: &HashMap<String, String>) -> Result<(), String> {
        let agy_ws = self.agy_workspace();
        let mut args = vec!["--add-dir".into(), agy_ws.to_string_lossy().into(), "--conversation".into(), session_id.into()];
        if self.config.permission_mode == "bypassPermissions" { args.push("--dangerously-skip-permissions".into()); }

        let (m, eff) = Self::resolve_cli_model_effort(self.config.model.as_deref(), self.config.effort.as_deref());
        if let Some(m) = m { args.extend(["--model".into(), m]); }
        if let Some(e) = eff { args.extend(["--effort".into(), e]); }
        args.extend(["--prompt-interactive".into(), "".into()]);
        self.sessions.ensure_session(session_id, &agy_ws, "agy", &args, env).await
    }

    async fn init_pty_if_needed(&self, sid: &str) -> Result<(), String> {
        let u = self.sessions.holders.lock().await.get_mut(sid)
            .map(|h| { let init = !h.initialized; h.initialized = true; init })
            .unwrap_or(false);
        if u {
            super::pty_spawner::wait_for_pty_prompt(&self.sessions, sid).await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        }
        Ok(())
    }

    pub async fn ensure_interactive_ready(&self, session_id: &str) -> Result<(), String> {
        if session_id.starts_with("mock-") {
            return Ok(());
        }
        let env = self.build_env();
        let agy_ws = self.agy_workspace();
        self.ensure_interactive_session(session_id, &agy_ws, &env).await?;
        self.init_pty_if_needed(session_id).await?;
        Ok(())
    }

    async fn run_in_pty_session(
        &self,
        session_id: &str,
        prompt: &str,
        env: &HashMap<String, String>,
        agy_ws: &Path,
    ) -> Result<(String, String, std::process::ExitStatus), String> {
        let sessions = self.sessions.clone();
        let sid_str = session_id.to_string();
        sessions.set_running(&sid_str, true).await;

        let res = async {
            if !self.sessions.is_active(session_id).await && !session_id.starts_with("mock-") && !self.is_session_alive(session_id) {
                return Err(format!("Session {} is expired or not found in Antigravity storage", session_id));
            }
            self.ensure_interactive_session(session_id, agy_ws, env).await?;
            self.init_pty_if_needed(session_id).await?;
            let p = events::agy_state_root(Some(env)).join("brain").join(session_id)
                .join(".system_generated/logs/transcript_full.jsonl");
            let sz = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            if let Some(h) = self.sessions.holders.lock().await.get_mut(session_id) {
                h.turn_start_offset = h.output.lock().await.len();
            }
            self.sessions.write_to_session(session_id, &format!("{}\r", prompt)).await?;
            super::polling::wait_for_log_completion(&self.sessions, session_id, Some(p), sz).await?;
            use std::os::unix::process::ExitStatusExt;
            Ok((String::new(), String::new(), std::process::ExitStatus::from_raw(0)))
        }.await;

        sessions.set_running(&sid_str, false).await;
        if let Err(ref e) = res {
            if e != "Interrupted by user (/stop)" && !e.starts_with("AGY_ERROR:") {
                sessions.terminate(&sid_str).await;
            }
        }
        if !sessions.is_active(&sid_str).await {
            sessions.set_ask_active(&sid_str, false).await;
        }
        res
    }

    async fn run_oneshot(
        &self,
        cmd_args: &[String],
        env: &HashMap<String, String>,
        workspace: &Path,
    ) -> Result<(String, String, std::process::ExitStatus), String> {
        let mut cmd = Command::new("agy");
        cmd.args(&cmd_args[1..])
            .current_dir(workspace)
            .envs(env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);

        let child = cmd.spawn().map_err(|e| format!("Failed to spawn agy command: {}", e))?;
        #[cfg(unix)]
        let child_pid = child.id();
        let (duration, display_secs) = Self::resolve_oneshot_timeout(cmd_args);

        match tokio::time::timeout(duration, child.wait_with_output()).await {
            Ok(Ok(o)) => Ok((String::from_utf8_lossy(&o.stdout).into(), String::from_utf8_lossy(&o.stderr).into(), o.status)),
            Ok(Err(e)) => Err(format!("Failed to wait for agy: {}", e)),
            Err(_) => {
                #[cfg(unix)]
                if let Some(pid) = child_pid {
                    let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(-(pid as i32)), nix::sys::signal::Signal::SIGKILL);
                }
                let label = if display_secs == 0 { duration.as_secs() } else { display_secs };
                Err(format!("Command timed out after {} seconds", label))
            }
        }
    }

    fn resolve_result_text(&self, ws: &Path, env: &HashMap<String, String>, stdout: &str, brain: Option<&Path>) -> String {
        events::read_transcript_answer(ws, Some(env), brain).unwrap_or_else(|| events::parse_antigravity_json(stdout))
    }
}

#[async_trait]
impl AgentProvider for AntigravityCli {
    async fn send(
        &self,
        prompt: &str,
        resume_session: Option<&str>,
        continue_session: bool,
        ws: PathBuf,
    ) -> Result<CliResponse, String> {
        let env = self.build_env();
        let agy_ws = if ws.as_os_str().is_empty() || ws == self.config.working_dir { self.agy_workspace() } else { ws };
        super::trust::trust_workspace_in_settings(&agy_ws, None);

        let (stdout_str, stderr_str, status) = if let Some(sid) = resume_session {
            self.run_in_pty_session(sid, prompt, &env, &agy_ws).await?
        } else {
            let args = self.build_command(prompt, resume_session, continue_session);
            self.run_oneshot(&args, &env, &agy_ws).await?
        };

        let final_sid = resume_session.map(str::to_string).or_else(|| {
            events::extract_conversation_id(&stdout_str).or_else(|| {
                events::resolve_brain_dir(&agy_ws, Some(&env))?.file_name().map(|n| n.to_string_lossy().into())
            })
        });
        let brain = final_sid.as_ref().map(|s| events::agy_state_root(Some(&env)).join("brain").join(s))
            .or_else(|| events::resolve_brain_dir(&agy_ws, Some(&env)));
        let result = self.resolve_result_text(&agy_ws, &env, &stdout_str, brain.as_deref());

        if resume_session.is_none() && std::sync::Arc::strong_count(&self.sessions) > 1 {
            if let Some(ref fid) = final_sid {
                self.ensure_interactive_session(fid, &agy_ws, &env).await?;
            }
        }

        Ok(CliResponse {
            session_id: final_sid,
            result,
            is_error: !status.success(),
            returncode: super::completion::exit_status_code(&status),
            stderr: stderr_str,
        })
    }

    async fn send_streaming<'a>(
        &'a self,
        prompt: &str,
        resume_session: Option<&str>,
        continue_session: bool,
        workspace: PathBuf,
    ) -> Result<BoxStream<'a, StreamEvent>, String> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let self_arc = std::sync::Arc::new(self.clone());
        let prompt_str = prompt.to_string();
        let resume_id = resume_session.map(str::to_string);
        let env = self.build_env();
        let initial_size = resume_id.as_ref().map(|sid| {
            let p = events::agy_state_root(Some(&env)).join("brain").join(sid).join(".system_generated/logs/transcript_full.jsonl");
            std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
        }).unwrap_or(0);
        let rid = resume_id.clone();
        let handle = tokio::spawn(async move { self_arc.send(&prompt_str, rid.as_deref(), continue_session, workspace).await });
        super::polling::spawn_log_polling(handle, tx, self.agy_workspace(), env, Some(initial_size), resume_id);
        Ok(Box::pin(futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|ev| (ev, rx)) })))
    }
}

