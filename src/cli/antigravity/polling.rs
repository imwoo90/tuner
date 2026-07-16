//! # Antigravity CLI Log Polling
//!
//! This module spawns background tasks to poll the transcript log
//! of active Antigravity CLI sessions and feed streaming progress.

use crate::cli::{CliResponse, StreamEvent};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use notify::{Watcher, RecommendedWatcher, RecursiveMode, EventKind};

fn err_resp(msg: String) -> CliResponse {
    CliResponse { session_id: None, result: msg.clone(), is_error: true, returncode: None, stderr: msg }
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
        Ok(Err(e)) => { let _ = tx.send(StreamEvent::Result(err_resp(e))); }
        Err(e) => { let _ = tx.send(StreamEvent::Result(err_resp(format!("Join error: {}", e)))); }
    }
}

fn parse_and_stream(
    ws: &Path,
    env: &HashMap<String, String>,
    act_path: &mut Option<PathBuf>,
    p_size: &mut Option<u64>,
    parser: &mut super::log_parser::AntigravityLogParser,
    tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) {
    if let Some(bd) = super::events::resolve_brain_dir(ws, Some(env)) {
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
    parse_and_stream(&agy_ws, &env, &mut active_path, &mut prev_size, &mut parser, &tx);

    loop {
        tokio::select! {
            res = &mut oneshot_handle => {
                handle_oneshot_finish(res, &tx);
                break;
            }
            _ = fs_rx.recv() => {
                parse_and_stream(&agy_ws, &env, &mut active_path, &mut prev_size, &mut parser, &tx);
            }
            _ = fallback.tick() => {
                parse_and_stream(&agy_ws, &env, &mut active_path, &mut prev_size, &mut parser, &tx);
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
) {
    tokio::spawn(poll_loop_async(oneshot_handle, tx, agy_ws, env, initial_size));
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

async fn check_completion_step(
    sessions: &super::session::SessionManager,
    session_id: &str,
    path: Option<&PathBuf>,
    size: &mut u64,
) -> Result<Option<()>, String> {
    let mut holders = sessions.holders.lock().await;
    let is_alive = if let Some(h) = holders.get_mut(session_id) {
        match h.child.try_wait() {
            Ok(None) => true,
            Ok(Some(s)) => {
                if s.success() {
                    if let Some(p) = path {
                        if check_log_completion_in_file(p, *size)?.is_none() {
                            return Ok(Some(()));
                        }
                    }
                    return Ok(Some(()));
                }
                return Err(format!("Process exited with error status: {:?}", s));
            }
            Err(e) => return Err(format!("Failed to check status: {}", e)),
        }
    } else {
        false
    };
    if !is_alive {
        return Err("Process exited prematurely".to_string());
    }
    if let Some(p) = path {
        match check_log_completion_in_file(p, *size)? {
            None => return Ok(Some(())),
            Some(ns) => *size = ns,
        }
    }
    Ok(None)
}

pub(crate) async fn wait_for_log_completion(
    sessions: &super::session::SessionManager,
    session_id: &str,
    transcript_path: Option<PathBuf>,
    mut current_size: u64,
) -> Result<(), String> {
    let timeout = tokio::time::Duration::from_secs(300);
    let start = std::time::Instant::now();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let _watcher = setup_path_watcher(transcript_path.as_ref(), tx);
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(500));

    while start.elapsed() < timeout {
        if let Some(()) = check_completion_step(sessions, session_id, transcript_path.as_ref(), &mut current_size).await? {
            return Ok(());
        }
        tokio::select! {
            _ = rx.recv() => {}
            _ = interval.tick() => {}
        }
    }
    Err("Timed out waiting for completion".to_string())
}

fn check_log_completion_in_file(path: &std::path::Path, current_size: u64) -> Result<Option<u64>, String> {
    if let Ok(metadata) = std::fs::metadata(path) {
        let file_size = metadata.len();
        if file_size > current_size {
            if let Ok(content) = std::fs::read_to_string(path) {
                let mut parser_pos = 0;
                for line in content.lines() {
                    let bytes_len = line.len() + 1;
                    if parser_pos >= current_size {
                        if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
                            let source = entry.get("source").and_then(|s| s.as_str());
                            let etype = entry.get("type").and_then(|s| s.as_str());
                            let status = entry.get("status").and_then(|s| s.as_str());
                            let tool_calls_empty = match entry.get("tool_calls") {
                                None => true,
                                Some(serde_json::Value::Array(arr)) => arr.is_empty(),
                                _ => false,
                            };
                            if source == Some("MODEL")
                                && etype == Some("PLANNER_RESPONSE")
                                && status == Some("DONE")
                                && tool_calls_empty
                            {
                                return Ok(None);
                            }
                        }
                    }
                    parser_pos += bytes_len as u64;
                }
            }
            return Ok(Some(file_size));
        }
    }
    Ok(Some(current_size))
}
