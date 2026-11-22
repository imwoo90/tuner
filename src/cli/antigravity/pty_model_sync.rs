//! # Antigravity Warm PTY Model & Default-Reset Synchronizer (`pty_model_sync.rs`)
//!
//! ## Overview
//! Synchronizes `/model <target>` directives onto ready interactive `agy` PTYs and guards
//! `antigravity-default` PTY resets against active or parked subagent trees, Native Swarms,
//! live timers/tasks, and child OS processes.
//!
//! ## Search Tags
//! #pty-model-sync, #default-reset-guard, #subagent-guard, #native-swarm

use super::AntigravityCli;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

impl AntigravityCli {
    pub(crate) fn desired_pty_target(&self) -> Option<String> {
        let raw_m = self.config.model.as_deref()?;
        super::model_spec::format_pty_model_target(raw_m, self.config.effort.as_deref(), None)
    }

    async fn finalize_pty_init(&self, sid: &str, pid: u32) -> Option<(u32, Option<String>)> {
        if !self.sessions.is_active(sid).await {
            self.pty_sync.remove_applied(sid).await;
            return None;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut holders = self.sessions.holders.lock().await;
        let Some(h) = holders.get_mut(sid) else {
            self.pty_sync.remove_applied(sid).await;
            return None;
        };
        if h.child.try_wait().ok().flatten().is_some() || h.child.id() != Some(pid) {
            return None;
        }
        h.initialized = true;
        drop(holders);
        self.pty_sync.set_applied(sid, pid, None).await;
        Some((pid, None))
    }

    /// Caller MUST hold `self.pty_sync.lock_for(sid)`. Waits for `wait_for_pty_prompt` before
    /// flipping `h.initialized = true`, cleaning up unready timed-out holders on failure.
    pub(crate) async fn init_locked_pty(
        &self,
        sid: &str,
    ) -> Result<Option<(u32, Option<String>)>, String> {
        if !self.sessions.is_active(sid).await {
            self.pty_sync.remove_applied(sid).await;
            return Ok(None);
        }
        let snap = self
            .sessions
            .holders
            .lock()
            .await
            .get(sid)
            .and_then(|h| Some((h.child.id()?, h.initialized)));
        let Some((pid, already_init)) = snap else {
            self.pty_sync.remove_applied(sid).await;
            return Ok(None);
        };
        if !already_init {
            if let Err(e) = super::pty_spawner::wait_for_pty_prompt(&self.sessions, sid).await {
                if !self.sessions.is_running(sid).await && !self.sessions.is_ask_active(sid).await {
                    self.terminate_session_clean(sid).await;
                }
                return Err(e);
            }
            return Ok(self.finalize_pty_init(sid, pid).await);
        }
        Ok(Some((pid, self.pty_sync.get_applied(sid, pid).await)))
    }

    pub(crate) async fn ensure_pty_turn_idle(&self, sid: &str, prompt: &str) -> Result<(), String> {
        if !prompt.is_empty()
            && (self.sessions.is_running(sid).await || self.sessions.is_ask_active(sid).await)
        {
            return Err("AGY_ERROR: Session already has an active turn or open question.".to_string());
        }
        Ok(())
    }

    /// Returns `true` only when `sid` holds a non-default model state and has zero live/parked
    /// subagents, Native Swarm members, active tasks/timers, or child OS processes.
    pub async fn can_reset_pty_to_default(&self, sid: &str) -> bool {
        if !self.sessions.is_active(sid).await {
            self.pty_sync.remove_applied(sid).await;
            return false;
        }
        let Some(pid) = self
            .sessions
            .holders
            .lock()
            .await
            .get(sid)
            .and_then(|h| h.child.id())
        else {
            return false;
        };
        if !self.pty_sync.has_custom_model(sid, pid).await
            || crate::upgrade::has_session_descendant_processes(pid)
        {
            return false;
        }
        let brain_root = super::events::agy_state_root(Some(&self.build_env())).join("brain");
        let ack = self
            .config
            .heartbeat
            .ack_token
            .as_deref()
            .unwrap_or("HEARTBEAT_OK");
        !crate::upgrade::brain_inspector::has_live_or_parked_agy_descendants(
            &brain_root,
            sid,
            crate::upgrade::activity::now_epoch_secs(),
            ack,
        )
    }

    pub(crate) async fn prepare_pty_for_turn(
        &self,
        sid: &str,
        prompt: &str,
        agy_ws: &Path,
        env: &HashMap<String, String>,
    ) -> Result<(), String> {
        let lock = self.pty_sync.lock_for(sid).await;
        let _guard = lock.lock().await;
        if !prompt.is_empty() {
            let t0 = std::time::Instant::now();
            while self.sessions.is_async_running(sid).await && t0.elapsed() < Duration::from_secs(30) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if self.sessions.is_async_running(sid).await || self.sessions.is_ask_active(sid).await {
                return Err("AGY_ERROR: Session already has an open prompt or async turn.".to_string());
            }
        }
        let desired = self.desired_pty_target();
        if !prompt.is_empty() && desired.is_none() && self.can_reset_pty_to_default(sid).await {
            self.terminate_session_clean(sid).await;
        }
        self.ensure_interactive_session(sid, agy_ws, env).await?;
        let state = self.init_locked_pty(sid).await?;
        if !prompt.is_empty()
            && let (Some((pid, applied)), Some(target)) = (state, desired)
            && applied.as_deref() != Some(target.as_str())
        {
            self.sessions
                .write_to_session(sid, &format!("/model {target}\r"))
                .await?;
            tokio::time::sleep(Duration::from_millis(300)).await;
            self.pty_sync.set_applied(sid, pid, Some(target)).await;
        }
        Ok(())
    }

    /// Caller MUST hold `self.pty_sync.lock_for(sid)`. Synchronizes `desired` (`None` for
    /// `"antigravity-default"`) onto an active idle PTY only when no turn or `ask_question` is active.
    pub async fn sync_locked_idle_pty(
        &self,
        sid: &str,
        chat_id: i64,
        topic_id: Option<i64>,
        desired: Option<String>,
    ) {
        if sid.is_empty()
            || sid.starts_with("mock-")
            || !self.sessions.is_active(sid).await
            || self.is_pty_busy_or_asking(sid, chat_id, topic_id).await
        {
            return;
        }
        let Some(target) = desired else {
            if self.can_reset_pty_to_default(sid).await {
                self.terminate_session_clean(sid).await;
            }
            return;
        };
        let Ok(Some((pid, applied))) = self.init_locked_pty(sid).await else {
            return;
        };
        if self.is_pty_busy_or_asking(sid, chat_id, topic_id).await
            || applied.as_deref() == Some(target.as_str())
        {
            return;
        }
        if self
            .sessions
            .write_to_session(sid, &format!("/model {target}\r"))
            .await
            .is_ok()
        {
            tokio::time::sleep(Duration::from_millis(300)).await;
            self.pty_sync.set_applied(sid, pid, Some(target)).await;
        }
    }

    pub(crate) async fn exec_pty_turn_inner(
        &self,
        sid: &str,
        prompt: &str,
        env: &HashMap<String, String>,
        agy_ws: &Path,
        p_tp: &Path,
    ) -> Result<(String, String, std::process::ExitStatus), String> {
        if !self.sessions.is_active(sid).await && !sid.starts_with("mock-") && !self.is_session_alive(sid) {
            return Err(format!("Session {} is expired or not found in Antigravity storage", sid));
        }
        self.prepare_pty_for_turn(sid, prompt, agy_ws, env).await?;
        let pre_bg = super::turn_settle::settle_before_pty_prompt(&self.sessions, sid, p_tp, prompt).await?;
        let sz = std::fs::metadata(p_tp).map(|m| m.len()).unwrap_or(0);
        self.pty_sync.set_turn_ctx(sid, sz, pre_bg).await;
        if let Some(h) = self.sessions.holders.lock().await.get_mut(sid) {
            h.turn_start_offset = h.output.lock().await.len();
        }
        let safe_p = super::pty_sync::sanitize_pty_turn_prompt(prompt);
        self.sessions.write_to_session(sid, &format!("{safe_p}\r")).await?;
        let p_opt = (!safe_p.is_empty()).then_some(safe_p.as_str());
        super::polling::wait_for_log_completion_with_prompt(&self.sessions, sid, Some(p_tp.to_path_buf()), sz, p_opt).await?;
        use std::os::unix::process::ExitStatusExt;
        Ok((String::new(), String::new(), std::process::ExitStatus::from_raw(0)))
    }

    pub(crate) async fn resolve_result_text(&self, ws: &Path, env: &HashMap<String, String>, stdout: &str, final_sid: Option<&str>, ok: bool) -> String {
        if !ok { return super::events::parse_antigravity_json(stdout); }
        let b = final_sid.map(|s| super::events::agy_state_root(Some(env)).join("brain").join(s))
            .or_else(|| super::events::resolve_brain_dir(ws, Some(env)));
        let base = super::events::read_transcript_answer(ws, Some(env), b.as_deref());
        if let (Some(sid), Some(bd)) = (final_sid, b.as_deref())
            && let Some((t_start, pre_bg)) = self.pty_sync.take_turn_ctx(sid).await
        {
            let tp = bd.join(".system_generated/logs/transcript_full.jsonl");
            if let Some(combined) = super::turn_settle::combine_turn_answers(&tp, t_start, pre_bg, base.clone()) {
                return combined;
            }
        }
        base.unwrap_or_else(|| super::events::parse_antigravity_json(stdout))
    }
}
