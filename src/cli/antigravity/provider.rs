//! # Antigravity CLI Provider
//!
//! ## Overview
//! Implements the AgentProvider trait for AntigravityCli, managing execution environments, PTY lifecycles, and logs.
//! Coordinates session resumption, one-shot execution, and live streaming channels across active workspaces.
//!
//! ## Search Tags
//! #antigravity-provider, #agent-provider, #pty-lifecycle

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
    pub(crate) async fn ensure_interactive_session(&self, session_id: &str, _workspace: &Path, env: &HashMap<String, String>) -> Result<(), String> {
        let agy_ws = self.agy_workspace();
        let mut args = vec!["--add-dir".into(), agy_ws.to_string_lossy().into(), "--conversation".into(), session_id.into()];
        if self.config.permission_mode == "bypassPermissions" { args.push("--dangerously-skip-permissions".into()); }
        args.extend(["--prompt-interactive".into(), "".into()]);
        self.sessions.ensure_session(session_id, &agy_ws, "agy", &args, env).await
    }

    pub(crate) async fn ensure_interactive_ready_locked(&self, session_id: &str) -> Result<(), String> {
        if session_id.starts_with("mock-") { return Ok(()); }
        super::model_spec::ensure_live_catalog_fresh().await;
        let env = self.build_env();
        self.ensure_interactive_session(session_id, &self.agy_workspace(), &env).await?;
        let state = self.init_locked_pty(session_id).await?;
        if let (Some((pid, applied)), Some(target)) = (state, self.desired_pty_target())
            && applied.as_deref() != Some(target.as_str())
        {
            self.sessions.write_to_session(session_id, &format!("/model {target}\r")).await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            self.pty_sync.set_applied(session_id, pid, Some(target)).await;
        }
        Ok(())
    }

    /// Executes `ensure_interactive_ready` (ensure interactive ready).
    pub async fn ensure_interactive_ready(&self, session_id: &str) -> Result<(), String> {
        let lock = self.pty_sync.lock_for(session_id).await;
        let _g = lock.lock().await;
        self.ensure_interactive_ready_locked(session_id).await
    }

    pub(crate) async fn run_in_pty_session(
        &self,
        sid: &str,
        prompt: &str,
        env: &HashMap<String, String>,
        agy_ws: &Path,
    ) -> Result<(String, String, std::process::ExitStatus), String> {
        self.ensure_pty_turn_idle(sid, prompt).await?;
        let sessions = self.sessions.clone();
        sessions.set_running(sid, true).await;
        let p_tp = events::agy_state_root(Some(env)).join("brain").join(sid)
            .join(".system_generated/logs/transcript_full.jsonl");
        let res = self.exec_pty_turn_inner(sid, prompt, env, agy_ws, &p_tp).await;
        let end_sz = std::fs::metadata(&p_tp).map(|m| m.len()).unwrap_or(0);
        sessions.set_running(sid, false).await;
        if res.is_ok() {
            self.pty_sync.record_fg_end(sid, end_sz).await;
        } else {
            let _ = self.pty_sync.take_turn_ctx(sid).await;
        }
        if let Err(ref e) = res
            && e != "Interrupted by user (/stop)" && !e.starts_with("AGY_ERROR:")
        {
            self.terminate_session_clean(sid).await;
        }
        if !sessions.is_active(sid).await {
            self.pty_sync.remove_applied(sid).await;
            sessions.set_ask_active(sid, false).await;
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
        cmd.args(&cmd_args[1..]).current_dir(workspace).envs(env)
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);

        let child = cmd.spawn().map_err(|e| format!("Failed to spawn agy command: {}", e))?;
        let child_pid = child.id();
        let (duration, display_secs) = Self::resolve_oneshot_timeout(cmd_args);

        match tokio::time::timeout(duration, child.wait_with_output()).await {
            Ok(Ok(o)) => Ok((String::from_utf8_lossy(&o.stdout).into(), String::from_utf8_lossy(&o.stderr).into(), o.status)),
            Ok(Err(e)) => Err(format!("Failed to wait for agy: {}", e)),
            Err(_) => {
                if let Some(pid) = child_pid { crate::cli::common::kill_process_group(pid); }
                let label = if display_secs == 0 { duration.as_secs() } else { display_secs };
                Err(format!("Command timed out after {} seconds", label))
            }
        }
    }

    async fn maybe_seed_warm_pty(&self, ok: bool, resume: Option<&str>, fid: Option<&str>, ws: &Path, env: &HashMap<String, String>) -> Result<(), String> {
        if ok && !self.oneshot && resume.is_none() && std::sync::Arc::strong_count(&self.sessions) > 1
            && let Some(sid) = fid
        {
            self.ensure_interactive_session(sid, ws, env).await?;
            let pid = self.sessions.holders.lock().await.get(sid).and_then(|h| h.child.id());
            self.pty_sync.record_seeded_target(sid, pid, self.desired_pty_target().is_some()).await;
        }
        Ok(())
    }
}

#[async_trait]
impl AgentProvider for AntigravityCli {
    async fn send(&self, prompt: &str, resume_session: Option<&str>, continue_session: bool, ws: PathBuf) -> Result<CliResponse, String> {
        match self.canonical_provider() {
            "claude" => return crate::cli::claude::send(self, prompt, resume_session, continue_session, ws).await,
            "codex" => return crate::cli::codex::send(self, prompt, resume_session, continue_session, ws).await,
            _ => {}
        }
        super::model_spec::ensure_live_catalog_fresh().await;
        let env = self.build_env();
        let agy_ws = if ws.as_os_str().is_empty() || ws == self.config.working_dir { self.agy_workspace() } else { ws };
        super::trust::trust_workspace_in_settings(&agy_ws, None);
        let (out, err, status) = if !self.oneshot
            && let Some(sid) = resume_session
        {
            self.run_in_pty_session(sid, prompt, &env, &agy_ws).await?
        } else {
            self.run_oneshot(&self.build_command(prompt, resume_session, continue_session), &env, &agy_ws).await?
        };
        let early = events::extract_finished_reason(&out).is_some();
        let ok = status.success() && !early;
        let final_sid = resume_session.map(str::to_string).or_else(|| {
            events::extract_conversation_id(&out).or_else(|| {
                ok.then(|| events::resolve_brain_dir(&agy_ws, Some(&env))?.file_name().map(|n| n.to_string_lossy().into()))?
            })
        });
        let result = self.resolve_result_text(&agy_ws, &env, &out, final_sid.as_deref(), ok).await;
        self.maybe_seed_warm_pty(ok, resume_session, final_sid.as_deref(), &agy_ws, &env).await?;
        let returncode = if early && status.success() { Some(1) } else { super::completion::exit_status_code(&status) };
        Ok(CliResponse { session_id: final_sid, result, is_error: !ok, returncode, stderr: err })
    }

    async fn send_streaming<'a>(
        &'a self,
        prompt: &str,
        resume_session: Option<&str>,
        continue_session: bool,
        workspace: PathBuf,
    ) -> Result<BoxStream<'a, StreamEvent>, String> {
        match self.canonical_provider() {
            "claude" => return crate::cli::claude::send_streaming(self, prompt, resume_session, continue_session, workspace).await,
            "codex" => return crate::cli::codex::send_streaming(self, prompt, resume_session, continue_session, workspace).await,
            _ => {}
        }
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

