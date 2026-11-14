//! # OpenAI Codex Turn Runner (runner.rs)
//!
//! ## Overview
//! Executes user turns via `turn/start` against the topic's persistent warm `codex app-server --stdio`
//! process, streaming notifications through `CodexStreamParser` and handling soft interruptions.
//!
//! ## Search Tags
//! #codex-runner, #stream-unfold, #warm-turn

use super::session::{acquire_codex_session, adjust_ephemeral_runs, terminate_topic, CodexProcessIo, WarmCodexSession};
use super::spawner::prepare_codex_turn_rpc;
use super::CodexStreamParser;
use crate::cli::antigravity::AntigravityCli;
use crate::cli::{CliResponse, StreamEvent};
use futures::stream::BoxStream;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

async fn handle_codex_ask_request(
    warm: &WarmCodexSession,
    val: &serde_json::Value,
    allow_ask: bool,
    parser: &mut CodexStreamParser,
    tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) -> Option<(bool, bool)> {
    let rpc_id = val.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let params = val.get("params").unwrap_or(&serde_json::Value::Null);
    let (questions, q_ids) = crate::cli::common::parse_codex_ask_questions(params);
    if !allow_ask {
        let reply = crate::cli::common::build_codex_ask_rpc_response(rpc_id, &q_ids, &[]);
        let _ = warm.write_rpc(&reply).await;
        return Some((true, false));
    }
    if questions.is_empty() {
        return None;
    }
    *warm.pending_ask.lock().await = Some((rpc_id, q_ids));
    warm.running.store(false, Ordering::SeqCst);
    if tx.send(StreamEvent::AskQuestion(questions)).is_err() {
        *warm.pending_ask.lock().await = None;
        warm.interrupted.store(true, Ordering::SeqCst);
    }
    parser.assistant_text.clear();
    parser.turn_completed = true;
    Some((true, true))
}

async fn handle_codex_rpc_line(
    warm: &WarmCodexSession,
    line: &str,
    ctx: (u64, bool),
    parser: &mut CodexStreamParser,
    tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) -> (bool, bool) {
    let (req_id, allow_ask) = ctx;
    let Ok(val) = serde_json::from_str::<serde_json::Value>(line.trim()) else { return (false, false); };
    if val.get("method").and_then(|v| v.as_str()) == Some("item/tool/requestUserInput")
        && let Some(out) = handle_codex_ask_request(warm, &val, allow_ask, parser, tx).await
    {
        return out;
    }
    if req_id > 0 && val.get("id").and_then(|v| v.as_u64()) == Some(req_id) {
        if let Some(tid) = val.get("result").and_then(|r| r.get("turn")).and_then(|t| t.get("id")).and_then(|v| v.as_str()) {
            *warm.active_turn_id.lock().await = Some(tid.to_string());
            if warm.interrupted.load(Ordering::SeqCst) {
                warm.send_turn_interrupt(tid).await;
                return (true, true);
            }
        } else if let Some(err) = val.get("error").and_then(|e| e.get("message")).and_then(|v| v.as_str()) {
            parser.error_message = Some(err.to_string());
            return (true, true);
        }
        return (true, false);
    }
    (false, false)
}

async fn read_codex_turn_stream(
    warm: &Arc<WarmCodexSession>,
    io: &mut CodexProcessIo,
    ctx: (u64, bool),
    parser: &mut CodexStreamParser,
    tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) {
    while !warm.interrupted.load(Ordering::SeqCst) {
        tokio::select! {
            biased;
            _ = warm.interrupt_notify.notified() => break,
            _ = tx.closed() => {
                if let Some(tid) = warm.active_turn_id.lock().await.clone() { warm.send_turn_interrupt(&tid).await; }
                warm.interrupted.store(true, Ordering::SeqCst);
                break;
            }
            line_res = io.stdout.next_line() => {
                let Ok(Some(line)) = line_res else { break; };
                let (handled, should_break) = handle_codex_rpc_line(warm, &line, ctx, parser, tx).await;
                if should_break { break; }
                if handled { continue; }
                for ev in parser.parse_line(&line) { let _ = tx.send(ev); }
                if parser.turn_completed { break; }
            }
        }
    }
}

async fn drain_after_interrupt(warm: &WarmCodexSession, io: &mut CodexProcessIo, req_id: u64) {
    let dl = tokio::time::Instant::now() + tokio::time::Duration::from_millis(2500);
    while let Ok(Ok(Some(line))) = tokio::time::timeout_at(dl, io.stdout.next_line()).await {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line.trim())
            && val.get("id").and_then(|v| v.as_u64()) == Some(req_id)
            && let Some(tid) = val.get("result").and_then(|r| r.get("turn")).and_then(|t| t.get("id")).and_then(|v| v.as_str())
        {
            warm.send_turn_interrupt(tid).await;
        }
        if line.contains("\"turn/completed\"") { break; }
    }
}

async fn emit_codex_final_result(
    topic: (i64, Option<i64>, bool),
    warm: &WarmCodexSession,
    io: &mut CodexProcessIo,
    req_id: u64,
    parser: CodexStreamParser,
    tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) {
    let (cid, tid, ephemeral) = topic;
    let _ = tokio::time::timeout(std::time::Duration::ZERO, warm.interrupt_notify.notified()).await;
    if warm.interrupted.swap(false, Ordering::SeqCst) {
        let had_ask = warm.pending_ask.lock().await.take().is_some();
        let _ = tx.send(StreamEvent::Result(CliResponse {
            session_id: Some(warm.thread_id.clone()), result: String::new(), is_error: true,
            returncode: Some(130), stderr: "Interrupted by user (/stop)".to_string(),
        }));
        if !ephemeral {
            if had_ask {
                let _ = terminate_topic(cid, tid, Some(&warm.thread_id)).await;
            } else {
                drain_after_interrupt(warm, io, req_id).await;
            }
        }
        return;
    }
    if !parser.turn_completed && !ephemeral {
        let _ = terminate_topic(cid, tid, Some(&warm.thread_id)).await;
    }
    let is_err = !parser.turn_completed || (parser.assistant_text.is_empty() && parser.error_message.is_some());
    let stderr = if !parser.turn_completed { "Warm codex app-server exited unexpectedly".to_string() } else { String::new() };
    let result = if !parser.assistant_text.is_empty() {
        parser.assistant_text
    } else {
        parser.error_message.unwrap_or_else(|| stderr.clone())
    };
    let _ = tx.send(StreamEvent::Result(CliResponse {
        session_id: Some(warm.thread_id.clone()), result, is_error: is_err,
        returncode: Some(if is_err { 1 } else { 0 }), stderr,
    }));
}

fn emit_error_event(
    tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
    sid: Option<String>,
    err: String,
) {
    let _ = tx.send(StreamEvent::Result(CliResponse {
        session_id: sid,
        result: err.clone(),
        is_error: true,
        returncode: Some(1),
        stderr: err,
    }));
}

async fn finish_ephemeral_or_broken(
    cli: &AntigravityCli,
    warm: &WarmCodexSession,
    ephemeral: bool,
    broken: bool,
) {
    if ephemeral {
        adjust_ephemeral_runs(false);
        warm.kill_now();
    } else if broken {
        let _ = terminate_topic(cli.config.chat_id, cli.config.topic_id, None).await;
    }
}

async fn execute_warm_codex_turn(
    cli: AntigravityCli,
    prompt: String,
    workspace: PathBuf,
    resume_sid: Option<String>,
    flags: (bool, bool),
    tx: tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) {
    let (continue_session, interactive) = flags;
    let (warm, ephemeral) = match acquire_codex_session(&cli, workspace, resume_sid.as_deref(), continue_session, prompt.is_empty()).await {
        Ok(pair) => pair,
        Err(msg) => return emit_error_event(&tx, resume_sid, msg),
    };
    if ephemeral { adjust_ephemeral_runs(true); }
    let mut io = warm.io.lock().await;
    let _ = tokio::time::timeout(std::time::Duration::ZERO, warm.interrupt_notify.notified()).await;
    warm.interrupted.store(false, Ordering::SeqCst);
    warm.running.store(true, Ordering::SeqCst);
    let (req_id, turn_req) = prepare_codex_turn_rpc(&warm, &cli, &prompt).await;
    if let Err(err) = warm.write_rpc(&turn_req).await {
        warm.running.store(false, Ordering::SeqCst);
        finish_ephemeral_or_broken(&cli, &warm, ephemeral, true).await;
        return emit_error_event(&tx, Some(warm.thread_id.clone()), err);
    }
    let mut parser = CodexStreamParser { session_id: Some(warm.thread_id.clone()), ..Default::default() };
    read_codex_turn_stream(&warm, &mut io, (req_id, interactive && !ephemeral), &mut parser, &tx).await;
    warm.running.store(false, Ordering::SeqCst);
    if warm.pending_ask.lock().await.is_none() {
        *warm.active_turn_id.lock().await = None;
    }
    emit_codex_final_result((cli.config.chat_id, cli.config.topic_id, ephemeral), &warm, &mut io, req_id, parser, &tx).await;
    drop(io);
    finish_ephemeral_or_broken(&cli, &warm, ephemeral, false).await;
}

/// Streams a `codex` CLI turn using the topic's persistent warm `app-server` process.
pub async fn send_streaming<'a>(
    cli: &'a AntigravityCli, prompt: &str, resume: Option<&str>, cont: bool, workspace: PathBuf,
) -> Result<BoxStream<'a, StreamEvent>, String> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(execute_warm_codex_turn(cli.clone(), prompt.into(), workspace, resume.map(Into::into), (cont, true), tx));
    Ok(Box::pin(futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|ev| (ev, rx)) })))
}

/// Runs a `codex` CLI turn to completion on the persistent warm `app-server` process.
pub async fn send(
    cli: &AntigravityCli, prompt: &str, resume: Option<&str>, cont: bool, workspace: PathBuf,
) -> Result<CliResponse, String> {
    use futures::StreamExt;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(execute_warm_codex_turn(cli.clone(), prompt.into(), workspace, resume.map(Into::into), (cont, false), tx));
    let (mut stream, mut res) = (Box::pin(futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|ev| (ev, rx)) })), None);
    while let Some(ev) = stream.next().await {
        if let StreamEvent::Result(r) = ev { res = Some(r); }
    }
    res.ok_or_else(|| "Codex CLI stream ended without a result".to_string())
}
