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
        let mut env = HashMap::new();
        for (k, v) in std::env::vars() {
            if k != "CODEX_SANDBOX_NETWORK_DISABLED" {
                env.insert(k, v);
            }
        }
        
        env.insert("DUCTOR_AGENT_NAME".to_string(), "main".to_string());
        env.insert("DUCTOR_CHAT_ID".to_string(), self.config.chat_id.to_string());
        if let Some(topic_id) = self.config.topic_id {
            env.insert("DUCTOR_TOPIC_ID".to_string(), topic_id.to_string());
        }
        env.insert("DUCTOR_TRANSPORT".to_string(), self.config.transport.clone());

        env.insert("DUCTOR_HOME".to_string(), "/home/wimvm/.ductor".to_string());
        env.insert("DUCTOR_SHARED_MEMORY_PATH".to_string(), "/home/wimvm/.ductor/SHAREDMEMORY.md".to_string());

        env
    }

    async fn ensure_interactive_session(
        &self,
        session_id: &str,
        _workspace: &Path,
        env: &HashMap<String, String>,
    ) -> Result<(), String> {
        let agy_ws = self.agy_workspace();
        let mut cmd_args = vec![
            "--add-dir".to_string(),
            agy_ws.to_string_lossy().to_string(),
            "--conversation".to_string(),
            session_id.to_string(),
        ];
        if self.config.permission_mode == "bypassPermissions" {
            cmd_args.push("--dangerously-skip-permissions".to_string());
        }
        cmd_args.push("--prompt-interactive".to_string());
        cmd_args.push("".to_string());

        self.sessions
            .ensure_session(session_id, &agy_ws, "agy", &cmd_args, env)
            .await
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

        let child = cmd.spawn()
            .map_err(|e| format!("Failed to spawn agy command: {}", e))?;

        let timeout_dur = tokio::time::Duration::from_secs(300);
        match tokio::time::timeout(timeout_dur, child.wait_with_output()).await {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                Ok((stdout, stderr, output.status))
            }
            Ok(Err(e)) => Err(format!("Failed to wait for agy: {}", e)),
            Err(_) => {
                Err("Command timed out after 300 seconds".to_string())
            }
        }
    }

    fn resolve_result_text(
        &self,
        agy_ws: &Path,
        env: &HashMap<String, String>,
        stdout_str: &str,
        brain_dir: Option<&Path>,
    ) -> String {
        let transcript_answer = events::read_transcript_answer(agy_ws, Some(env), brain_dir);
        match transcript_answer {
            Some(ans) => ans,
            None => events::parse_antigravity_json(stdout_str),
        }
    }

    async fn handle_session_transition(
        &self,
        resume_session: Option<&str>,
        final_session_id: Option<&str>,
        workspace: &Path,
        env: &HashMap<String, String>,
    ) -> Result<(), String> {
        if let Some(final_id) = final_session_id {
            if let Some(resume_id) = resume_session {
                if resume_id != final_id {
                    self.sessions.terminate(resume_id).await;
                }
            }
            self.ensure_interactive_session(final_id, workspace, env).await?;
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

        if let Some(session_id) = resume_session {
            self.ensure_interactive_session(session_id, &agy_ws, &env).await?;
        }

        let cmd_args = self.build_command(prompt, resume_session, continue_session);
        let (stdout_str, stderr_str, status) = self.run_oneshot(&cmd_args, &env, &agy_ws).await?;

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
                let transcript_path = brain_dir.join(".system_generated").join("logs").join("transcript.jsonl");
                std::fs::metadata(&transcript_path)
                    .map(|m| m.len())
                    .unwrap_or(0)
            });

        let oneshot_handle = tokio::spawn(async move {
            self_arc.send(&prompt_string, resume_id.as_deref(), continue_session, workspace_path).await
        });

        spawn_log_polling(oneshot_handle, tx, self.agy_workspace(), self.build_env(), initial_size);

        let stream = futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|event| (event, rx))
        });
        Ok(Box::pin(stream))
    }
}

fn handle_oneshot_finish(
    res: Result<Result<CliResponse, String>, tokio::task::JoinError>,
    tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) {
    match res {
        Ok(Ok(resp)) => {
            let _ = tx.send(StreamEvent::TextDelta(resp.result.clone()));
            let _ = tx.send(StreamEvent::Result(resp));
        }
        Ok(Err(e)) => {
            let _ = tx.send(StreamEvent::Result(CliResponse {
                session_id: None,
                result: e.clone(),
                is_error: true,
                returncode: None,
                stderr: e,
            }));
        }
        Err(e) => {
            let err_msg = format!("Join error: {}", e);
            let _ = tx.send(StreamEvent::Result(CliResponse {
                session_id: None,
                result: err_msg.clone(),
                is_error: true,
                returncode: None,
                stderr: err_msg,
            }));
        }
    }
}

fn spawn_log_polling(
    mut oneshot_handle: tokio::task::JoinHandle<Result<CliResponse, String>>,
    tx: tokio::sync::mpsc::UnboundedSender<StreamEvent>,
    agy_ws: PathBuf,
    env: HashMap<String, String>,
    initial_size: Option<u64>,
) {
    tokio::spawn(async move {
        let mut prev_size = initial_size;
        let mut active_path: Option<PathBuf> = None;
        let mut parser = super::log_parser::AntigravityLogParser::new();
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(500));
        loop {
            tokio::select! {
                res = &mut oneshot_handle => {
                    handle_oneshot_finish(res, &tx);
                    break;
                }
                _ = interval.tick() => {
                    if let Some(brain_dir) = events::resolve_brain_dir(&agy_ws, Some(&env)) {
                        let transcript_path = brain_dir.join(".system_generated").join("logs").join("transcript.jsonl");
                        if let Some(ref old_path) = active_path {
                            if old_path != &transcript_path {
                                prev_size = None;
                                parser = super::log_parser::AntigravityLogParser::new();
                            }
                        }
                        active_path = Some(transcript_path.clone());
                        let (new_size, delta_text) = parser.parse_log_delta(&transcript_path, prev_size);
                        prev_size = Some(new_size);
                        if let Some(text) = delta_text {
                            let _ = tx.send(StreamEvent::TextDelta(text));
                        }
                    }
                }
            }
        }
    });
}
