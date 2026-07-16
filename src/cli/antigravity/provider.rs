//! # Antigravity CLI Provider implementation
//!
//! This module implements the `AgentProvider` trait for `AntigravityCli`,
//! managing execution environment propagation, PTY session lifecycle hooks,
//! and response transcript extraction.

use crate::cli::antigravity::AntigravityCli;
use crate::cli::{AgentProvider, CliResponse, StreamEvent};
use crate::cli::antigravity::events;
use async_trait::async_trait;
use futures::stream::BoxStream;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

impl AntigravityCli {
    pub(crate) fn build_env(&self) -> HashMap<String, String> {
        let mut env: HashMap<_, _> = std::env::vars().filter(|(k, _)| k != "CODEX_SANDBOX_NETWORK_DISABLED").collect();
        let mut add = |k: &str, v: String| { env.insert(k.into(), v); };
        add("TUNER_AGENT_NAME", "main".into());
        add("TUNER_CHAT_ID", self.config.chat_id.to_string());
        if let Some(tid) = self.config.topic_id {
            add("TUNER_TOPIC_ID", tid.to_string());
        }
        add("TUNER_TRANSPORT", self.config.transport.clone());
        add("TUNER_HOME", "/home/wimvm/.tuner".into());
        add("TUNER_SHARED_MEMORY_PATH", "/home/wimvm/.tuner/SHAREDMEMORY.md".into());
        env
    }

    async fn ensure_interactive_session(
        &self,
        session_id: &str,
        _workspace: &Path,
        env: &HashMap<String, String>,
    ) -> Result<(), String> {
        let agy_ws = self.agy_workspace();
        let mut args = vec!["--add-dir".into(), agy_ws.to_string_lossy().into(), "--conversation".into(), session_id.into()];
        if self.config.permission_mode == "bypassPermissions" {
            args.push("--dangerously-skip-permissions".into());
        }
        args.extend(vec!["--prompt-interactive".into(), "".into()]);
        self.sessions.ensure_session(session_id, &agy_ws, "agy", &args, env).await
    }

    async fn run_in_pty_session(
        &self,
        session_id: &str,
        prompt: &str,
        env: &HashMap<String, String>,
        agy_ws: &Path,
    ) -> Result<(String, String, std::process::ExitStatus), String> {
        let was_running = {
            let holders = self.sessions.holders.lock().await;
            holders.contains_key(session_id)
        };

        let sessions = self.sessions.clone();
        let sid_str = session_id.to_string();
        sessions.set_running(&sid_str, true).await;

        let res = async {
            self.ensure_interactive_session(session_id, agy_ws, env).await?;

            if !was_running {
                wait_for_pty_prompt(&self.sessions, session_id).await?;
                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            }

            let resolved_brain_dir = events::resolve_brain_dir(agy_ws, Some(env));
            let transcript_path = resolved_brain_dir.as_ref().map(|d| {
                d.join(".system_generated").join("logs").join("transcript_full.jsonl")
            });

            let current_size = transcript_path.as_ref()
                .and_then(|p| std::fs::metadata(p).ok())
                .map(|m| m.len())
                .unwrap_or(0);

            let input_prompt = format!("{}\r", prompt);
            self.sessions.write_to_session(session_id, &input_prompt).await?;

            super::polling::wait_for_log_completion(&self.sessions, session_id, transcript_path, current_size).await?;

            use std::os::unix::process::ExitStatusExt;
            let status = std::process::ExitStatus::from_raw(0);
            Ok((String::new(), String::new(), status))
        }.await;

        sessions.set_running(&sid_str, false).await;
        sessions.set_ask_active(&sid_str, false).await;
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

        let child = cmd.spawn()
            .map_err(|e| format!("Failed to spawn agy command: {}", e))?;

        let task_id = if self.config.process_label.starts_with("task:") {
            Some(self.config.process_label["task:".len()..].to_string())
        } else {
            None
        };

        if let Some(ref tid) = task_id {
            if let Some(pid) = child.id() {
                if let Some(registry) = crate::tasks::runner::GLOBAL_PROCESS_REGISTRY.get() {
                    registry.register(tid.clone(), pid).await;
                }
            }
        }

        let timeout_dur = tokio::time::Duration::from_secs(300);
        let res = match tokio::time::timeout(timeout_dur, child.wait_with_output()).await {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                Ok((stdout, stderr, output.status))
            }
            Ok(Err(e)) => Err(format!("Failed to wait for agy: {}", e)),
            Err(_) => {
                Err("Command timed out after 300 seconds".to_string())
            }
        };

        if let Some(ref tid) = task_id {
            if let Some(registry) = crate::tasks::runner::GLOBAL_PROCESS_REGISTRY.get() {
                registry.unregister(tid).await;
            }
        }

        res
    }

    fn resolve_result_text(&self, ws: &Path, env: &HashMap<String, String>, stdout: &str, brain: Option<&Path>) -> String {
        events::read_transcript_answer(ws, Some(env), brain)
            .unwrap_or_else(|| events::parse_antigravity_json(stdout))
    }

    async fn handle_session_transition(
        &self,
        resume: Option<&str>,
        final_id: Option<&str>,
        ws: &Path,
        env: &HashMap<String, String>,
    ) -> Result<(), String> {
        if let Some(fid) = final_id {
            if resume.filter(|&r| r != fid).is_some() {
                self.sessions.terminate(resume.unwrap()).await;
            }
            self.ensure_interactive_session(fid, ws, env).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl AgentProvider for AntigravityCli {
    async fn send(
        &self,
        prompt: &str,
        resume_session: Option<&str>,
        continue_session: bool,
        _workspace: PathBuf,
    ) -> Result<CliResponse, String> {
        let env = self.build_env();
        let agy_ws = self.agy_workspace();

        crate::cli::antigravity::trust::trust_workspace_in_settings(&agy_ws, None);

        let (stdout_str, stderr_str, status) = if let Some(session_id) = resume_session {
            self.run_in_pty_session(session_id, prompt, &env, &agy_ws).await?
        } else {
            let cmd_args = self.build_command(prompt, resume_session, continue_session);
            self.run_oneshot(&cmd_args, &env, &agy_ws).await?
        };

        let resolved_brain_dir = events::resolve_brain_dir(&agy_ws, Some(&env));
        let final_session_id = resolved_brain_dir
            .as_ref()
            .and_then(|d| d.file_name())
            .map(|name| name.to_string_lossy().to_string());

        let result_text = self.resolve_result_text(&agy_ws, &env, &stdout_str, resolved_brain_dir.as_deref());

        self.handle_session_transition(resume_session, final_session_id.as_deref(), &agy_ws, &env).await?;

        let is_error = !status.success();
        Ok(CliResponse {
            session_id: final_session_id,
            result: result_text,
            is_error,
            returncode: status.code(),
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
        let prompt_string = prompt.to_string();
        let resume_id = resume_session.map(|s| s.to_string());
        let workspace_path = workspace.clone();

        let env = self.build_env();
        let agy_ws = self.agy_workspace();
        let initial_size = events::resolve_brain_dir(&agy_ws, Some(&env))
            .map(|brain_dir| {
                let transcript_path = brain_dir.join(".system_generated").join("logs").join("transcript_full.jsonl");
                std::fs::metadata(&transcript_path)
                    .map(|m| m.len())
                    .unwrap_or(0)
            });

        let oneshot_handle = tokio::spawn(async move {
            self_arc.send(&prompt_string, resume_id.as_deref(), continue_session, workspace_path).await
        });

        super::polling::spawn_log_polling(oneshot_handle, tx, self.agy_workspace(), self.build_env(), initial_size);

        let stream = futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|event| (event, rx))
        });
        Ok(Box::pin(stream))
    }
}

async fn wait_for_pty_prompt(
    mgr: &super::session::SessionManager,
    sid: &str,
) -> Result<(), String> {
    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < 15 {
        let (out, dead) = {
            let mut hs = mgr.holders.lock().await;
            if let Some(h) = hs.get_mut(sid) {
                (h.output.lock().await.clone(), h.child.try_wait().ok().flatten().is_some())
            } else {
                break;
            }
        };
        let s = String::from_utf8_lossy(&out);
        println!("🤖 [tuner] PTY Output so far: {:?}", s);
        if s.contains("\r> ") || s.contains("\n> ") || dead {
            return Ok(());
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    Err("Timeout waiting for interactive session initialization".to_string())
}



