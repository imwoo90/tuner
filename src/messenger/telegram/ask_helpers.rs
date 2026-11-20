//! # Ask-Question Execution Helper Utilities
//!
//! ## Overview
//! Provides utilities to query active sessions, feed user input to PTY stdin channels,
//! and clean up prompt-waiting locks upon resolution.
//!
//! ## Search Tags
//! #ask-helpers

use teloxide::prelude::*;
use crate::cli::antigravity::AntigravityCli;
use crate::session::manager::SessionManager;
use crate::session::data::SessionData;
use crate::config::CliConfig;
use crate::cli::antigravity::session::AskState;
use std::sync::Arc;
use super::multi_select::{build_multi_select_keyboard, build_single_select_keyboard};

pub(crate) fn find_write_in_index(options: &[String]) -> usize {
    for (i, opt) in options.iter().enumerate() {
        let lower = opt.to_lowercase();
        if lower.contains("write-in") || lower.contains("직접 입력") {
            return i;
        }
    }
    options.len()
}

fn determine_input_mode(
    current_text: &str,
    q: &crate::cli::AskQuestionData,
    waiting_for_write_in: bool,
) -> (String, bool) {
    let mut ip = String::new();
    let mut is_write_in = false;
    if waiting_for_write_in {
        is_write_in = true;
    } else if let Some(i) = super::formatting::find_best_option(current_text, &q.options) {
        let opt = &q.options[i];
        if opt.to_lowercase().contains("write-in") || opt.contains("직접 입력") {
            is_write_in = true;
        } else {
            ip = format!("{}", i + 1);
        }
    } else {
        is_write_in = true;
    }
    (ip, is_write_in)
}

async fn send_write_in_input(
    sid: &str,
    current_text: &str,
    w_idx: usize,
    prev_text: &str,
    is_multi_select: bool,
    cli: &AntigravityCli,
) -> String {
    let _ = cli.sessions.write_to_session(sid, &format!("{}", w_idx + 1)).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
    
    if !prev_text.is_empty() {
        let backspaces = "\x7F".repeat(prev_text.chars().count());
        let _ = cli.sessions.write_to_session(sid, &backspaces).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
    }
    
    let ip = format!("{}\r", current_text);
    let _ = cli.sessions.write_to_session(sid, &ip).await;
    if is_multi_select {
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
        let _ = cli.sessions.write_to_session(sid, "\r").await;
    }
    ip
}

async fn send_predefined_input_with_clear(
    sid: &str,
    ip: &str,
    w_idx: usize,
    prev_text: &str,
    options_len: usize,
    cli: &AntigravityCli,
) {
    if !prev_text.is_empty() && w_idx <= options_len {
        let _ = cli.sessions.write_to_session(sid, &format!("{}", w_idx + 1)).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
        let backspaces = "\x7F".repeat(prev_text.chars().count());
        let _ = cli.sessions.write_to_session(sid, &format!("{}\r", backspaces)).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
        let _ = cli.sessions.write_to_session(sid, "\x1B[D").await;
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
    }
    let _ = cli.sessions.write_to_session(sid, ip).await;
}

async fn determine_and_send_input(
    sid: &str,
    current_text: &str,
    cli: &AntigravityCli,
    state: &crate::cli::antigravity::session::AskState,
) -> (String, bool) {
    let mut ip = String::new();
    let mut is_write_in = false;
    if let Some(q) = state.questions.get(state.current_index) {
        let (det_ip, det_is_write_in) = determine_input_mode(current_text, q, state.waiting_for_write_in);
        ip = det_ip;
        is_write_in = det_is_write_in;
        let w_idx = find_write_in_index(&q.options);
        let prev_text = state.answers.get(state.current_index).cloned().unwrap_or_default();

        if is_write_in {
            ip = send_write_in_input(sid, current_text, w_idx, &prev_text, q.is_multi_select, cli).await;
        } else {
            send_predefined_input_with_clear(sid, &ip, w_idx, &prev_text, q.options.len(), cli).await;
        }
    }
    (ip, is_write_in)
}

async fn send_next_question_below(
    bot: &teloxide::Bot,
    msg: &Message,
    q: &crate::cli::AskQuestionData,
    markup: teloxide::types::InlineKeyboardMarkup,
) -> (Option<i32>, bool, Option<String>) {
    let html = super::formatting::markdown_to_telegram_html(&q.question);
    let mut req = bot.send_message(msg.chat.id, html)
        .parse_mode(teloxide::types::ParseMode::Html)
        .reply_markup(markup);
    if let Some(tid) = msg.thread_id {
        req = req.message_thread_id(tid);
    }
    match req.await {
        Ok(sent) => (Some(sent.id.0), true, None),
        Err(e) => (None, false, Some(e.to_string())),
    }
}

async fn advance_ask_state(
    bot: &teloxide::Bot,
    msg: &Message,
    sid: &str,
    mut state: AskState,
    cli: &AntigravityCli,
    sess_ctx: (&Arc<SessionManager>, SessionData),
    config: &CliConfig,
) -> Result<(), teloxide::RequestError> {
    let (sessions, sess) = sess_ctx;
    let cid = msg.chat.id.0;
    if state.current_index + 1 < state.questions.len() {
        state.current_index += 1;
        let nq = state.questions[state.current_index].clone();
        let bmp = super::multi_select::get_bitmap_for_question(&nq, &state.answers, state.current_index);
        state.current_bitmap = bmp.clone();
        state.waiting_for_write_in = false;
        let (sent_id, ok, err) = send_next_question_below(bot, msg, &nq, build_ask_keyboard_helper(sid, &nq, &bmp, true)).await;
        if let Some(new_mid) = sent_id { state.msg_id = new_mid; }
        super::ask_process::log_ask_question_result(config, sid, (sess.topic_id, sess.topic_name.as_deref()), sent_id, &nq, (ok, err.as_deref()));
        cli.sessions.set_ask_state(sid, state).await;
        crate::messenger::web::events_bridge::tg_ask_by_sid(cid, sid, false, "").await;
    } else {
        let turn_guard = cli.begin_topic_turn(cid, sess.topic_id.or_else(|| super::get_topic_id(msg)));
        let summary = state.answers.last().cloned().unwrap_or_default();
        super::ask_callbacks::submit_peer_ask_answers_if_needed(cid, &sess, config, &state.answers).await;
        cli.sessions.set_ask_state(sid, state).await;
        cli.sessions.set_ask_active(sid, false).await;
        crate::messenger::web::events_bridge::tg_ask_by_sid(cid, sid, true, &summary).await;
        super::ask_callbacks::spawn_cli_stream_in_background(bot, msg, sid.to_string(), cli.clone(), (sessions.clone(), sess), config.clone(), turn_guard);
    }
    Ok(())
}

async fn handle_ask_input(
    bot: &teloxide::Bot,
    msg: &Message,
    session_id: &str,
    current_text: &str,
    cli_and_state: (&AntigravityCli, AskState),
    sess_ctx: (&Arc<SessionManager>, SessionData),
    config: &CliConfig,
) -> Result<String, teloxide::RequestError> {
    let (cli, mut state) = cli_and_state;
    let mid = state.msg_id;
    let (ip, _) = determine_and_send_input(session_id, current_text, cli, &state).await;

    if state.current_index < state.answers.len() {
        state.answers[state.current_index] = current_text.to_string();
    }

    if let Some(q) = state.questions.get(state.current_index) {
        let txt = format!("{}\n\n(Selected: **{}**)", q.question, current_text);
        let html = super::formatting::markdown_to_telegram_html(&txt);
        let _ = bot.edit_message_text(msg.chat.id, teloxide::types::MessageId(mid), html)
            .parse_mode(teloxide::types::ParseMode::Html)
            .await;
    }

    advance_ask_state(bot, msg, session_id, state, cli, sess_ctx, config).await?;
    Ok(ip)
}

pub(crate) async fn feed_active_session_if_running(
    bot: &teloxide::Bot,
    msg: &Message,
    session_id: &str,
    current_text: &str,
    cli: &AntigravityCli,
    sess_ctx: (&Arc<SessionManager>, SessionData),
    config: &CliConfig,
) -> Result<bool, teloxide::RequestError> {
    let prov = SessionData::effective_provider(&sess_ctx.1.provider, &config.provider);
    let (cid, tid) = (msg.chat.id.0, sess_ctx.1.topic_id);
    if cli.sessions.is_ask_active(session_id).await {
        if cli.is_ask_backing_alive(prov, cid, tid, session_id).await
            && let Some(state) = cli.sessions.get_ask_state(session_id).await
        {
            let _ = handle_ask_input(bot, msg, session_id, current_text, (cli, state), sess_ctx, config).await?;
            return Ok(true);
        }
        cli.sessions.set_ask_active(session_id, false).await;
    }

    let start = std::time::Instant::now();
    while ((cli.sessions.is_active(session_id).await && cli.sessions.is_running(session_id).await)
        || crate::cli::claude::is_running(cid, tid, Some(session_id)).await
        || crate::cli::codex::is_running(cid, tid, Some(session_id)).await)
        && start.elapsed().as_secs() < 60
    {
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
    }
    Ok(false)
}

pub(crate) fn build_ask_keyboard_helper(
    sid: &str,
    nq: &crate::cli::AskQuestionData,
    bitmap: &str,
    show_prev: bool,
) -> teloxide::types::InlineKeyboardMarkup {
    if nq.is_multi_select {
        build_multi_select_keyboard(sid, &nq.options, bitmap, show_prev)
    } else {
        build_single_select_keyboard(sid, &nq.options, show_prev)
    }
}

pub(crate) async fn update_telegram_ask_ui(bot: &teloxide::Bot, msg: &Message, idx: usize, data: &str) {
    let mut chosen = format!("Option {}", idx + 1);
    if let Some(rm) = msg.reply_markup() {
        for r in &rm.inline_keyboard {
            for b in r {
                if let teloxide::types::InlineKeyboardButtonKind::CallbackData(cbd) = &b.kind
                    && cbd == data { chosen = b.text.clone(); }
            }
        }
    }
    let txt = format!("{}\n\n(Selected: **{}**)", msg.text().unwrap_or(""), chosen);
    let html = super::formatting::markdown_to_telegram_html(&txt);
    let _ = bot.edit_message_text(msg.chat.id, msg.id, html).parse_mode(teloxide::types::ParseMode::Html).await;
}
