//! # Antigravity CLI Log Polling
//!
//! This module spawns background tasks to poll the transcript log
//! of active Antigravity CLI sessions and feed streaming progress.

//! 
//! ## Search Tags
//! #polling

use crate::cli::{CliResponse, StreamEvent};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use notify::{Watcher, RecommendedWatcher, RecursiveMode, EventKind};

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
            let _ = tx.send(StreamEvent::TextDelta(resp.result.clone()));
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
        let (ns, txt, ask) = parser.parse_log_delta(&tp, *p_size);
        *p_size = Some(ns);
        if let Some(t) = txt { let _ = tx.send(StreamEvent::TextDelta(t)); }
        if let Some(a) = ask { let _ = tx.send(StreamEvent::AskQuestion(a)); }
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
    let mut prev_size = initial_size;
    let mut active_path = None;
    let mut parser = super::log_parser::AntigravityLogParser::new();

    let (fs_tx, mut fs_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut watcher = None;
    let brain_root = super::events::agy_state_root(Some(&env)).join("brain");

    if let Ok(mut w) = notify::recommended_watcher(move |res: Result<notify::Event, _>| {
        if let Ok(e) = res {
            if matches!(e.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                let _ = fs_tx.send(());
            }
        }
    }) {
        if w.watch(&brain_root, RecursiveMode::Recursive).is_ok() {
            watcher = Some(w);
        }
    }

    let mut fallback = tokio::time::interval(tokio::time::Duration::from_secs(5));
    parse_and_stream(&agy_ws, &env, &mut active_path, &mut prev_size, &mut parser, &tx, session_id.as_deref());

    let mut fs_rx_closed = false;
    loop {
        tokio::select! {
            res = &mut oneshot_handle => {
                handle_oneshot_finish(res, &tx, session_id);
                break;
            }
            res = fs_rx.recv(), if !fs_rx_closed => {
                if res.is_some() {
                    parse_and_stream(&agy_ws, &env, &mut active_path, &mut prev_size, &mut parser, &tx, session_id.as_deref());
                } else {
                    fs_rx_closed = true;
                }
            }
            _ = fallback.tick() => {
                parse_and_stream(&agy_ws, &env, &mut active_path, &mut prev_size, &mut parser, &tx, session_id.as_deref());
            }
        }
    }
    drop(watcher);
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

fn setup_path_watcher(
    path: Option<&PathBuf>,
    tx: tokio::sync::mpsc::UnboundedSender<()>,
) -> Option<RecommendedWatcher> {
    let p = path?.parent()?;
    let mut w = notify::recommended_watcher(move |res: Result<notify::Event, _>| {
        if let Ok(e) = res {
            if matches!(e.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                let _ = tx.send(());
            }
        }
    }).ok()?;
    w.watch(p, RecursiveMode::NonRecursive).ok()?;
    Some(w)
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
    session_id: &str,
    transcript_path: Option<PathBuf>,
    mut current_size: u64,
) -> Result<(), String> {
    let inactivity_timeout = tokio::time::Duration::from_secs(300);
    let max_total_timeout = tokio::time::Duration::from_secs(3600);
    let start = std::time::Instant::now();
    let mut last_activity = std::time::Instant::now();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let _watcher = setup_path_watcher(transcript_path.as_ref(), tx);
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(500));

    let mut rx_closed = false;
    while last_activity.elapsed() < inactivity_timeout && start.elapsed() < max_total_timeout {
        check_interruption(sessions, session_id).await?;
        let prev_size = current_size;
        if let Some(()) = super::completion::check_completion_step(sessions, session_id, transcript_path.as_ref(), &mut current_size).await? {
            return Ok(());
        }
        if current_size > prev_size {
            last_activity = std::time::Instant::now();
        }
        tokio::select! {
            _ = sessions.interrupt_notify.notified() => {
                check_interruption(sessions, session_id).await?;
            }
            res = rx.recv(), if !rx_closed => {
                if res.is_none() {
                    rx_closed = true;
                } else {
                    last_activity = std::time::Instant::now();
                }
            }
            _ = interval.tick() => {}
        }
    }
    if start.elapsed() >= max_total_timeout {
        Err("Exceeded maximum execution duration of 3600s".to_string())
    } else {
        Err("Timed out waiting for completion (inactivity)".to_string())
    }
}

