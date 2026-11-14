//! # Claude Code Turn Runner (runner.rs)
//!
//! ## Overview
//! Executes user turns against the topic's persistent warm `claude` process, streaming
//! JSONL output frames through `ClaudeStreamParser` and handling soft interruptions.
//!
//! ## Search Tags
//! #claude-runner, #stream-unfold, #warm-turn

use super::session::{acquire_claude_session, adjust_ephemeral_runs, terminate_topic, ClaudeProcessIo, WarmClaudeSession};
use super::spawner::prepare_claude_turn_payload;
use super::ClaudeStreamParser;
use crate::cli::antigravity::AntigravityCli;
use crate::cli::{CliResponse, StreamEvent};
use futures::stream::BoxStream;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

async fn handle_claude_ask_tool(
    warm: &WarmClaudeSession,
    req_id: String,
    input: serde_json::Value,
    allow_ask: bool,
    parser: &mut ClaudeStreamParser,
    tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) -> bool {
    let questions = crate::cli::common::parse_claude_ask_questions(&input);
    if !allow_ask {
        let deny = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": req_id,
                "response": {
                    "behavior": "deny",
                    "message": "Interactive questions are unavailable in non-interactive mode; proceed autonomously."
                }
            }
        });
        let _ = warm.write_json_line(&deny).await;
        return true;
    }
    if questions.is_empty() {
        return false;
    }
    *warm.pending_ask.lock().await = Some((req_id, input));
    warm.running.store(false, Ordering::SeqCst);
    if tx.send(StreamEvent::AskQuestion(questions)).is_err() {
        *warm.pending_ask.lock().await = None;
        warm.interrupted.store(true, Ordering::SeqCst);
        return true;
    }
    let resp = CliResponse {
        session_id: parser.session_id.clone(),
        result: String::new(),
        is_error: false,
        returncode: Some(0),
        stderr: String::new(),
    };
    parser.final_result = Some(resp.clone());
    let _ = tx.send(StreamEvent::Result(resp));
    true
}

async fn handle_claude_control_request(
    warm: &WarmClaudeSession,
    line: &str,
    allow_ask: bool,
    parser: &mut ClaudeStreamParser,
    tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) -> bool {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(line.trim()) else { return false; };
    if val.get("type").and_then(|v| v.as_str()) != Some("control_request") {
        return false;
    }
    if let Some(sid) = val.get("session_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        parser.session_id = Some(sid.to_string());
        *warm.session_id.lock().await = Some(sid.to_string());
    }
    let req_id = val.get("request_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let req = val.get("request").unwrap_or(&serde_json::Value::Null);
    if req.get("subtype").and_then(|v| v.as_str()) == Some("can_use_tool") {
        let tool = req.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
        let input = req.get("input").cloned().unwrap_or_else(|| serde_json::json!({}));
        if tool == "AskUserQuestion"
            && handle_claude_ask_tool(warm, req_id.clone(), input.clone(), allow_ask, parser, tx).await
        {
            return true;
        }
        let allow = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": req_id,
                "response": { "behavior": "allow", "updatedInput": input }
            }
        });
        let _ = warm.write_json_line(&allow).await;
    }
    true
}

async fn read_claude_turn_stream(
    warm: &Arc<WarmClaudeSession>,
    io: &mut ClaudeProcessIo,
    allow_ask: bool,
    parser: &mut ClaudeStreamParser,
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
                let ctrl = serde_json::json!({
                    "type": "control_request",
                    "request_id": "stop-drop",
                    "request": { "subtype": "interrupt" }
                });
                let _ = warm.write_json_line(&ctrl).await;
                warm.interrupted.store(true, Ordering::SeqCst);
                break;
            }
            line_res = io.stdout.next_line() => {
                let Ok(Some(line)) = line_res else { break; };
                if handle_claude_control_request(warm, &line, allow_ask, parser, tx).await {
                    if parser.final_result.is_some() { break; }
                    continue;
                }
                for ev in parser.parse_line(&line) { let _ = tx.send(ev); }
                if let Some(ref sid) = parser.session_id {
                    *warm.session_id.lock().await = Some(sid.clone());
                }
                if parser.final_result.is_some() { break; }
            }
        }
    }
}

async fn finalize_claude_turn(
    cli: &AntigravityCli,
    warm: &WarmClaudeSession,
    ephemeral: bool,
    io: &mut ClaudeProcessIo,
    parser: ClaudeStreamParser,
    resume_sid: Option<String>,
    tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) {
    let _ = tokio::time::timeout(std::time::Duration::ZERO, warm.interrupt_notify.notified()).await;
    if warm.interrupted.swap(false, Ordering::SeqCst) {
        let had_ask = warm.pending_ask.lock().await.take().is_some();
        let _ = tx.send(StreamEvent::Result(CliResponse {
            session_id: parser.session_id.or(resume_sid),
            result: String::new(), is_error: true, returncode: Some(130),
            stderr: "Interrupted by user (/stop)".to_string(),
        }));
        if !ephemeral {
            if had_ask {
                let _ = terminate_topic(cli.config.chat_id, cli.config.topic_id, None).await;
            } else {
                io.drain_after_interrupt().await;
            }
        }
    } else if parser.final_result.is_none() {
        if !ephemeral {
            let _ = terminate_topic(cli.config.chat_id, cli.config.topic_id, None).await;
        }
        let _ = tx.send(StreamEvent::Result(CliResponse {
            session_id: parser.session_id.or(resume_sid),
            result: parser.assistant_text, is_error: true, returncode: Some(1),
            stderr: "Warm claude process exited unexpectedly".to_string(),
        }));
    }
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
    warm: &WarmClaudeSession,
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

async fn execute_warm_claude_turn(
    cli: AntigravityCli,
    prompt: String,
    workspace: PathBuf,
    resume_sid: Option<String>,
    flags: (bool, bool),
    tx: tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) {
    let (continue_session, interactive) = flags;
    let (warm, ephemeral) = match acquire_claude_session(&cli, workspace, resume_sid.as_deref(), continue_session, prompt.is_empty()).await {
        Ok(pair) => pair,
        Err(msg) => return emit_error_event(&tx, resume_sid, msg),
    };
    if ephemeral { adjust_ephemeral_runs(true); }
    let mut io = warm.io.lock().await;
    let _ = tokio::time::timeout(std::time::Duration::ZERO, warm.interrupt_notify.notified()).await;
    warm.interrupted.store(false, Ordering::SeqCst);
    warm.running.store(true, Ordering::SeqCst);
    let outbound = prepare_claude_turn_payload(&warm, &cli, &prompt).await;
    if let Err(err) = warm.write_json_line(&outbound).await {
        warm.running.store(false, Ordering::SeqCst);
        finish_ephemeral_or_broken(&cli, &warm, ephemeral, true).await;
        return emit_error_event(&tx, resume_sid, err);
    }
    let mut parser = ClaudeStreamParser { session_id: resume_sid.clone(), ..Default::default() };
    read_claude_turn_stream(&warm, &mut io, interactive && !ephemeral, &mut parser, &tx).await;
    warm.running.store(false, Ordering::SeqCst);
    finalize_claude_turn(&cli, &warm, ephemeral, &mut io, parser, resume_sid, &tx).await;
    drop(io);
    finish_ephemeral_or_broken(&cli, &warm, ephemeral, false).await;
}

/// Streams a `claude` CLI turn using the topic's persistent warm process.
pub async fn send_streaming<'a>(
    cli: &'a AntigravityCli,
    prompt: &str,
    resume_session: Option<&str>,
    continue_session: bool,
    workspace: PathBuf,
) -> Result<BoxStream<'a, StreamEvent>, String> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(execute_warm_claude_turn(
        cli.clone(), prompt.to_string(), workspace, resume_session.map(str::to_string), (continue_session, true), tx,
    ));
    Ok(Box::pin(futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|ev| (ev, rx)) })))
}

/// Runs a `claude` CLI turn to completion on the persistent warm process and returns `CliResponse`.
pub async fn send(
    cli: &AntigravityCli,
    prompt: &str,
    resume_session: Option<&str>,
    continue_session: bool,
    workspace: PathBuf,
) -> Result<CliResponse, String> {
    use futures::StreamExt;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(execute_warm_claude_turn(
        cli.clone(), prompt.to_string(), workspace, resume_session.map(str::to_string), (continue_session, false), tx,
    ));
    let mut stream = Box::pin(futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|ev| (ev, rx)) }));
    let mut last_result = None;
    while let Some(ev) = stream.next().await {
        if let StreamEvent::Result(resp) = ev { last_result = Some(resp); }
    }
    last_result.ok_or_else(|| "Claude CLI stream ended without a result".to_string())
}
