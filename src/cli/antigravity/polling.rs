//! # Antigravity CLI Log Polling
//!
//! ## Overview
//! This module spawns background tasks to poll the transcript log
//! of active Antigravity CLI sessions and feed streaming progress.
//!
//! ## Search Tags
//! #polling

use crate::cli::{CliResponse, StreamEvent};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
pub(crate) use super::log_watcher::{create_brain_dir_watcher, resolve_watch_target, setup_path_watcher, sync_nonrecursive_watcher};

fn extract_exit_code(msg: &str) -> Option<i32> {
    let lower = msg.to_lowercase();
    if let Some(pos) = lower.find("exit code") {
        let after = &lower[pos + "exit code".len()..];
        let num: String = after.trim_start_matches(|c: char| c == ':' || c.is_whitespace())
            .chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(code) = num.parse::<i32>() { return Some(code); }
    }
    if let Some(pos) = lower.find("exit status:") {
        let after = &lower[pos + "exit status:".len()..];
        let num: String = after.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(code) = num.parse::<i32>() { return Some(code); }
    }
    if let Some(pos) = lower.find("unix_wait_status(") {
        let after = &lower[pos + "unix_wait_status(".len()..];
        let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(raw) = num.parse::<i32>() { return Some((raw >> 8) & 0xff); }
    }
    None
}

fn err_resp(msg: String, session_id: Option<String>) -> CliResponse {
    let returncode = extract_exit_code(&msg);
    CliResponse { session_id, result: msg.clone(), is_error: true, returncode, stderr: msg }
}

fn handle_oneshot_finish(
    res: Result<Result<CliResponse, String>, tokio::task::JoinError>,
    tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
    session_id: Option<String>,
) {
    match res {
        Ok(Ok(resp)) => {
            if session_id.is_none() && !resp.is_error && !resp.result.is_empty() {
                let _ = tx.send(StreamEvent::TextDelta(resp.result.clone()));
            }
            let _ = tx.send(StreamEvent::Result(resp));
        }
        Ok(Err(e)) => { let _ = tx.send(StreamEvent::Result(err_resp(e, session_id))); }
        Err(e) => { let _ = tx.send(StreamEvent::Result(err_resp(format!("Join error: {}", e), session_id))); }
    }
}

fn parse_and_stream(
    ws: &Path,
    env: &HashMap<String, String>,
    act_path: &mut Option<PathBuf>,
    p_size: &mut Option<u64>,
    parser: &mut super::log_parser::AntigravityLogParser,
    tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
    session_id: Option<&str>,
) {
    let bd_opt = if let Some(sid) = session_id {
        Some(super::events::agy_state_root(Some(env)).join("brain").join(sid))
    } else {
        super::events::resolve_brain_dir(ws, Some(env))
    };

    if let Some(bd) = bd_opt {
        let tp = bd.join(".system_generated").join("logs").join("transcript_full.jsonl");
        if act_path.is_some() && Some(&tp) != act_path.as_ref() {
            *p_size = None;
            *parser = super::log_parser::AntigravityLogParser::new();
        }
        *act_path = Some(tp.clone());
        let res = parser.parse_log_delta_structured(&tp, *p_size);
        *p_size = Some(res.new_size);
        if res.final_content.is_none()
            && let Some(t) = res.formatted { let _ = tx.send(StreamEvent::TextDelta(t)); }
        if let Some(a) = res.ask_question { let _ = tx.send(StreamEvent::AskQuestion(a)); }
    }
}

struct PollState {
    prev_size: Option<u64>,
    active_path: Option<PathBuf>,
    parser: super::log_parser::AntigravityLogParser,
    watcher: Option<notify::RecommendedWatcher>,
    watched_dir: Option<PathBuf>,
}

impl PollState {
    fn step(
        &mut self,
        ws: &Path,
        env: &HashMap<String, String>,
        brain_root: &Path,
        tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
        fs_tx: &tokio::sync::mpsc::UnboundedSender<()>,
        sid: Option<&str>,
    ) {
        parse_and_stream(ws, env, &mut self.active_path, &mut self.prev_size, &mut self.parser, tx, sid);
        let tgt = resolve_watch_target(brain_root, sid, self.active_path.as_deref());
        sync_nonrecursive_watcher(&mut self.watcher, &mut self.watched_dir, tgt, fs_tx);
    }
}

async fn poll_loop_async(
    mut oneshot_handle: tokio::task::JoinHandle<Result<CliResponse, String>>,
    tx: tokio::sync::mpsc::UnboundedSender<StreamEvent>,
    agy_ws: PathBuf,
    env: HashMap<String, String>,
    initial_size: Option<u64>,
    session_id: Option<String>,
) {
    let mut st = PollState {
        prev_size: initial_size,
        active_path: None,
        parser: super::log_parser::AntigravityLogParser::new(),
        watcher: None,
        watched_dir: None,
    };
    let (fs_tx, mut fs_rx) = tokio::sync::mpsc::unbounded_channel();
    let brain_root = super::events::agy_state_root(Some(&env)).join("brain");
    let mut fallback = tokio::time::interval(tokio::time::Duration::from_millis(500));
    st.step(&agy_ws, &env, &brain_root, &tx, &fs_tx, session_id.as_deref());

    let mut fs_rx_closed = false;
    loop {
        tokio::select! {
            res = &mut oneshot_handle => {
                let drain_sid = session_id.as_deref().or_else(|| {
                    res.as_ref().ok().and_then(|r| r.as_ref().ok()).and_then(|c| c.session_id.as_deref())
                });
                parse_and_stream(&agy_ws, &env, &mut st.active_path, &mut st.prev_size, &mut st.parser, &tx, drain_sid);
                handle_oneshot_finish(res, &tx, session_id);
                break;
            }
            res = fs_rx.recv(), if !fs_rx_closed => {
                if res.is_some() {
                    st.step(&agy_ws, &env, &brain_root, &tx, &fs_tx, session_id.as_deref());
                } else {
                    fs_rx_closed = true;
                }
            }
            _ = fallback.tick() => {
                st.step(&agy_ws, &env, &brain_root, &tx, &fs_tx, session_id.as_deref());
            }
        }
    }
}

pub(crate) fn spawn_log_polling(
    oneshot_handle: tokio::task::JoinHandle<Result<CliResponse, String>>,
    tx: tokio::sync::mpsc::UnboundedSender<StreamEvent>,
    agy_ws: PathBuf,
    env: HashMap<String, String>,
    initial_size: Option<u64>,
    session_id: Option<String>,
) {
    tokio::spawn(poll_loop_async(oneshot_handle, tx, agy_ws, env, initial_size, session_id));
}

async fn check_interruption(sessions: &super::session::SessionManager, session_id: &str) -> Result<(), String> {
    if sessions.is_interrupted(session_id).await {
        sessions.clear_interrupted(session_id).await;
        return Err("Interrupted by user (/stop)".to_string());
    }
    Ok(())
}

pub(crate) async fn wait_for_log_completion(
    sessions: &super::session::SessionManager,
    sid: &str,
    tp: Option<PathBuf>,
    mut sz: u64,
) -> Result<(), String> {
    use std::time::{Duration, Instant};
    let (inact, max_t, start, mut last_act) = (Duration::from_secs(300), Duration::from_secs(3600), Instant::now(), Instant::now());
    let mut last_pty = sessions.get_output_len(sid).await.unwrap_or(0);
    let (mut t0, mut retried, mut closed) = (sz, false, false);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let _w = setup_path_watcher(tp.as_ref(), tx);
    let mut tick = tokio::time::interval(Duration::from_millis(500));

    while last_act.elapsed() < inact && start.elapsed() < max_t {
        check_interruption(sessions, sid).await?;
        let prev = sz;
        if super::completion::check_completion_step(sessions, sid, tp.as_ref(), &mut sz).await?.is_some() { return Ok(()); }
        let pty = sessions.get_output_len(sid).await.unwrap_or(last_pty);
        if sz > prev || pty > last_pty {
            (last_pty, last_act) = (pty, Instant::now());
        } else if last_act.elapsed() >= Duration::from_secs(3)
            && super::completion::handle_thinking_only_stall(sessions, sid, tp.as_ref(), &mut t0, sz, &mut retried).await?
        {
            last_act = Instant::now();
        }
        tokio::select! {
            _ = sessions.interrupt_notify.notified() => check_interruption(sessions, sid).await?,
            r = rx.recv(), if !closed => if r.is_none() { closed = true; } else { last_act = Instant::now(); },
            _ = tick.tick() => {}
        }
    }
    Err(if start.elapsed() >= max_t { "Exceeded maximum execution duration of 3600s".into() } else { "Timed out waiting for completion (inactivity)".into() })
}

