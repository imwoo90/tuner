//! # Claude Code Turn Runner (runner.rs)
//!
//! ## Overview
//! Executes user turns against the topic's persistent warm `claude` process, streaming
//! JSONL output frames through `ClaudeStreamParser` and handling soft interruptions.
//!
//! ## Search Tags
//! #claude-runner, #stream-unfold, #warm-turn

use super::session::{get_or_spawn_warm_session, terminate_topic, ClaudeProcessIo, WarmClaudeSession};
use super::ClaudeStreamParser;
use crate::cli::antigravity::AntigravityCli;
use crate::cli::{CliResponse, StreamEvent};
use futures::stream::BoxStream;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

async fn drain_after_interrupt(io: &mut ClaudeProcessIo) {
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(2500);
    while let Ok(Ok(Some(line))) = tokio::time::timeout_at(deadline, io.stdout.next_line()).await {
        if line.contains("\"type\":\"result\"") {
            break;
        }
    }
}

async fn handle_claude_control_request(
    warm: &WarmClaudeSession,
    line: &str,
    parser: &mut ClaudeStreamParser,
    tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) -> bool {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(line.trim()) else { return false; };
    if val.get("type").and_then(|v| v.as_str()) != Some("control_request") {
        return false;
    }
    let req_id = val.get("request_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let req = val.get("request").unwrap_or(&serde_json::Value::Null);
    if req.get("subtype").and_then(|v| v.as_str()) == Some("can_use_tool") {
        let tool = req.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
        let input = req.get("input").cloned().unwrap_or_else(|| serde_json::json!({}));
        if tool == "AskUserQuestion" {
            let questions = crate::cli::common::parse_claude_ask_questions(&input);
            if !questions.is_empty() {
                *warm.pending_ask.lock().await = Some((req_id, input));
                let _ = tx.send(StreamEvent::AskQuestion(questions));
                let resp = CliResponse {
                    session_id: parser.session_id.clone(),
                    result: String::new(),
                    is_error: false,
                    returncode: Some(0),
                    stderr: String::new(),
                };
                parser.final_result = Some(resp.clone());
                let _ = tx.send(StreamEvent::Result(resp));
                return true;
            }
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
                if handle_claude_control_request(warm, &line, parser, tx).await {
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
    io: &mut ClaudeProcessIo,
    parser: ClaudeStreamParser,
    resume_sid: Option<String>,
    tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) {
    let _ = tokio::time::timeout(std::time::Duration::ZERO, warm.interrupt_notify.notified()).await;
    if warm.interrupted.swap(false, Ordering::SeqCst) {
        let _ = tx.send(StreamEvent::Result(CliResponse {
            session_id: parser.session_id.or(resume_sid),
            result: String::new(), is_error: true, returncode: Some(130),
            stderr: "Interrupted by user (/stop)".to_string(),
        }));
        drain_after_interrupt(io).await;
    } else if parser.final_result.is_none() {
        let _ = terminate_topic(cli.config.chat_id, cli.config.topic_id, None).await;
        let _ = tx.send(StreamEvent::Result(CliResponse {
            session_id: parser.session_id.or(resume_sid),
            result: parser.assistant_text, is_error: true, returncode: Some(1),
            stderr: "Warm claude process exited unexpectedly".to_string(),
        }));
    }
}

async fn prepare_claude_turn_payload(warm: &WarmClaudeSession, cli: &AntigravityCli, prompt: &str) -> serde_json::Value {
    if prompt.is_empty()
        && let Some((req_id, input)) = warm.pending_ask.lock().await.take()
    {
        let ans = warm.queued_ask_answers.lock().await.take().unwrap_or_default();
        return crate::cli::common::build_claude_ask_control_response(&req_id, input, &ans);
    }
    serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": cli.format_prompt(prompt) }
    })
}

async fn execute_warm_claude_turn(
    cli: AntigravityCli,
    prompt: String,
    workspace: PathBuf,
    resume_sid: Option<String>,
    continue_session: bool,
    tx: tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) {
    let warm = match get_or_spawn_warm_session(&cli, workspace, resume_sid.as_deref(), continue_session).await {
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
    let outbound = prepare_claude_turn_payload(&warm, &cli, &prompt).await;
    if let Err(err) = warm.write_json_line(&outbound).await {
        warm.running.store(false, Ordering::SeqCst);
        let _ = terminate_topic(cli.config.chat_id, cli.config.topic_id, None).await;
        let _ = tx.send(StreamEvent::Result(CliResponse {
            session_id: resume_sid, result: err.clone(), is_error: true, returncode: Some(1), stderr: err,
        }));
        return;
    }
    let mut parser = ClaudeStreamParser { session_id: resume_sid.clone(), ..Default::default() };
    read_claude_turn_stream(&warm, &mut io, &mut parser, &tx).await;
    warm.running.store(false, Ordering::SeqCst);
    finalize_claude_turn(&cli, &warm, &mut io, parser, resume_sid, &tx).await;
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
        cli.clone(), prompt.to_string(), workspace, resume_session.map(str::to_string), continue_session, tx,
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
    let mut stream = send_streaming(cli, prompt, resume_session, continue_session, workspace).await?;
    let mut last_result = None;
    while let Some(ev) = stream.next().await {
        if let StreamEvent::Result(resp) = ev { last_result = Some(resp); }
    }
    last_result.ok_or_else(|| "Claude CLI stream ended without a result".to_string())
}
