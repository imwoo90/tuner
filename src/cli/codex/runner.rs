//! # OpenAI Codex Turn Runner (runner.rs)
//!
//! ## Overview
//! Executes user turns via `turn/start` against the topic's persistent warm `codex app-server --stdio`
//! process, streaming notifications through `CodexStreamParser` and handling soft interruptions.
//!
//! ## Search Tags
//! #codex-runner, #stream-unfold, #warm-turn

use super::session::{get_or_spawn_warm_codex, terminate_topic, CodexProcessIo, WarmCodexSession};
use super::{resolve_codex_model_effort, CodexStreamParser, DEFAULT_CODEX_MODEL};
use crate::cli::antigravity::AntigravityCli;
use crate::cli::{CliResponse, StreamEvent};
use futures::stream::BoxStream;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

async fn handle_codex_rpc_line(
    warm: &WarmCodexSession,
    line: &str,
    req_id: u64,
    parser: &mut CodexStreamParser,
    tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) -> (bool, bool) {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(line.trim()) else { return (false, false); };
    if val.get("method").and_then(|v| v.as_str()) == Some("item/tool/requestUserInput") {
        let rpc_id = val.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let params = val.get("params").unwrap_or(&serde_json::Value::Null);
        let (questions, q_ids) = crate::cli::common::parse_codex_ask_questions(params);
        if !questions.is_empty() {
            *warm.pending_ask.lock().await = Some((rpc_id, q_ids));
            let _ = tx.send(StreamEvent::AskQuestion(questions));
            parser.assistant_text.clear();
            parser.turn_completed = true;
            return (true, true);
        }
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
    req_id: u64,
    parser: &mut CodexStreamParser,
    tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) {
    loop {
        if warm.interrupted.load(Ordering::SeqCst) {
            break;
        }
        tokio::select! {
            biased;
            _ = warm.interrupt_notify.notified() => break,
            _ = tx.closed() => {
                if let Some(tid) = warm.active_turn_id.lock().await.clone() {
                    warm.send_turn_interrupt(&tid).await;
                }
                warm.interrupted.store(true, Ordering::SeqCst);
                break;
            }
            line_res = io.stdout.next_line() => {
                let Ok(Some(line)) = line_res else { break; };
                let (handled, should_break) = handle_codex_rpc_line(warm, &line, req_id, parser, tx).await;
                if should_break { break; }
                if handled { continue; }
                for ev in parser.parse_line(&line) { let _ = tx.send(ev); }
                if parser.turn_completed { break; }
            }
        }
    }
}

async fn drain_after_interrupt(warm: &WarmCodexSession, io: &mut CodexProcessIo, req_id: u64) {
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(2500);
    while let Ok(Ok(Some(line))) = tokio::time::timeout_at(deadline, io.stdout.next_line()).await {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line.trim())
            && val.get("id").and_then(|v| v.as_u64()) == Some(req_id)
            && let Some(tid) = val.get("result").and_then(|r| r.get("turn")).and_then(|t| t.get("id")).and_then(|v| v.as_str())
        {
            warm.send_turn_interrupt(tid).await;
        }
        if line.contains("\"turn/completed\"") {
            break;
        }
    }
}

async fn emit_codex_final_result(
    warm: &WarmCodexSession,
    io: &mut CodexProcessIo,
    req_id: u64,
    parser: CodexStreamParser,
    tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) {
    let _ = tokio::time::timeout(std::time::Duration::ZERO, warm.interrupt_notify.notified()).await;
    if warm.interrupted.swap(false, Ordering::SeqCst) {
        let _ = tx.send(StreamEvent::Result(CliResponse {
            session_id: Some(warm.thread_id.clone()), result: String::new(), is_error: true,
            returncode: Some(130), stderr: "Interrupted by user (/stop)".to_string(),
        }));
        drain_after_interrupt(warm, io, req_id).await;
        return;
    }
    let is_error = !parser.turn_completed || (parser.assistant_text.is_empty() && parser.error_message.is_some());
    let result = if !parser.assistant_text.is_empty() {
        parser.assistant_text
    } else {
        parser.error_message.unwrap_or_default()
    };
    let _ = tx.send(StreamEvent::Result(CliResponse {
        session_id: Some(warm.thread_id.clone()), result, is_error,
        returncode: Some(if is_error { 1 } else { 0 }), stderr: String::new(),
    }));
}

fn build_turn_start_rpc(cli: &AntigravityCli, thread_id: &str, req_id: u64, prompt: &str) -> serde_json::Value {
    let (m, eff) = resolve_codex_model_effort(cli.config.model.as_deref(), cli.config.effort.as_deref());
    serde_json::json!({
        "jsonrpc": "2.0", "id": req_id, "method": "turn/start",
        "params": {
            "threadId": thread_id,
            "input": [{ "type": "text", "text": cli.format_prompt(prompt) }],
            "model": m.unwrap_or_else(|| DEFAULT_CODEX_MODEL.to_string()),
            "effort": eff.unwrap_or_else(|| "medium".to_string()),
        }
    })
}

async fn prepare_codex_turn_rpc(warm: &WarmCodexSession, cli: &AntigravityCli, prompt: &str) -> (u64, serde_json::Value) {
    if prompt.is_empty()
        && let Some((rpc_id, q_ids)) = warm.pending_ask.lock().await.take()
    {
        let ans = warm.queued_ask_answers.lock().await.take().unwrap_or_default();
        return (0, crate::cli::common::build_codex_ask_rpc_response(rpc_id, &q_ids, &ans));
    }
    let id = warm.next_req_id.fetch_add(1, Ordering::SeqCst);
    (id, build_turn_start_rpc(cli, &warm.thread_id, id, prompt))
}

async fn execute_warm_codex_turn(
    cli: AntigravityCli,
    prompt: String,
    workspace: PathBuf,
    resume_sid: Option<String>,
    continue_session: bool,
    tx: tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) {
    let warm = match get_or_spawn_warm_codex(&cli, workspace, resume_sid.as_deref(), continue_session).await {
        Ok(w) => w,
        Err(msg) => {
            let _ = tx.send(StreamEvent::Result(CliResponse {
                session_id: resume_sid, result: msg.clone(), is_error: true, returncode: Some(1), stderr: msg,
            }));
            return;
        }
    };
    let mut io = warm.io.lock().await;
    let _ = tokio::time::timeout(std::time::Duration::ZERO, warm.interrupt_notify.notified()).await;
    warm.interrupted.store(false, Ordering::SeqCst);
    warm.running.store(true, Ordering::SeqCst);
    let (req_id, turn_req) = prepare_codex_turn_rpc(&warm, &cli, &prompt).await;
    if let Err(err) = warm.write_rpc(&turn_req).await {
        warm.running.store(false, Ordering::SeqCst);
        let _ = terminate_topic(cli.config.chat_id, cli.config.topic_id, None).await;
        let _ = tx.send(StreamEvent::Result(CliResponse {
            session_id: Some(warm.thread_id.clone()), result: err.clone(), is_error: true, returncode: Some(1), stderr: err,
        }));
        return;
    }
    let mut parser = CodexStreamParser { session_id: Some(warm.thread_id.clone()), ..Default::default() };
    read_codex_turn_stream(&warm, &mut io, req_id, &mut parser, &tx).await;
    warm.running.store(false, Ordering::SeqCst);
    if warm.pending_ask.lock().await.is_none() {
        *warm.active_turn_id.lock().await = None;
    }
    emit_codex_final_result(&warm, &mut io, req_id, parser, &tx).await;
}

/// Streams a `codex` CLI turn using the topic's persistent warm `app-server` process.
pub async fn send_streaming<'a>(
    cli: &'a AntigravityCli,
    prompt: &str,
    resume_session: Option<&str>,
    continue_session: bool,
    workspace: PathBuf,
) -> Result<BoxStream<'a, StreamEvent>, String> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(execute_warm_codex_turn(
        cli.clone(), prompt.to_string(), workspace, resume_session.map(str::to_string), continue_session, tx,
    ));
    Ok(Box::pin(futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|ev| (ev, rx)) })))
}

/// Runs a `codex` CLI turn to completion on the persistent warm `app-server` process.
pub async fn send(
    cli: &AntigravityCli,
    prompt: &str,
    resume_session: Option<&str>,
    continue_session: bool,
    workspace: PathBuf,
) -> Result<CliResponse, String> {
    use futures::StreamExt;
    let mut stream = send_streaming(cli, prompt, resume_session, continue_session, workspace).await?;
    let mut last_result = None;
    while let Some(ev) = stream.next().await {
        if let StreamEvent::Result(resp) = ev { last_result = Some(resp); }
    }
    last_result.ok_or_else(|| "Codex CLI stream ended without a result".to_string())
}
