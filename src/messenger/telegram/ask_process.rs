//! # Interactive Ask and Question Handler Process
//!
//! ## Overview
//! Orchestrates the interactive answer submission flow for agent prompts (`ask_question`).
//! Feeds responses back to background execution PTY streams and handles state transitions.
//!
//! ## Search Tags
//! #ask-process, #user-input, #pty-feedback

use teloxide::prelude::*;
use teloxide::types::{MessageId, ParseMode, ThreadId};
use crate::config::CliConfig;
use crate::cli::{AskQuestionData, antigravity::{AntigravityCli, session::AskState}};
use crate::session::{data::SessionData, manager::SessionManager};
use super::multi_select::{build_multi_select_keyboard, build_single_select_keyboard};
use super::ask_helpers::find_write_in_index;
use super::ask_callbacks::{spawn_cli_stream_in_background, submit_peer_ask_answers_if_needed};

pub(crate) async fn clear_previous_write_in_if_any(
    sid: &str,
    cli: &AntigravityCli,
    state: &AskState,
    should_clear: bool,
) {
    let prev = state.answers.get(state.current_index).cloned().unwrap_or_default();
    let Some(q) = state.questions.get(state.current_index).filter(|q| should_clear && !prev.is_empty() && !q.options.contains(&prev)) else {
        return;
    };
    let w_idx = find_write_in_index(&q.options);
    if w_idx <= q.options.len() {
        let bs = "\x7F".repeat(prev.chars().count());
        for s in [format!("{}", w_idx + 1), format!("{}\r", bs), "\x1B[D".to_string()] {
            let _ = cli.sessions.write_to_session(sid, &s).await;
            tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
        }
    }
}

pub(crate) async fn advance_ask_index_or_finish(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: MessageId,
    sid: &str,
    cli: &AntigravityCli,
    state: &mut AskState,
) -> Result<bool, teloxide::RequestError> {
    if state.current_index + 1 < state.questions.len() {
        state.current_index += 1;
        let nq = &state.questions[state.current_index];
        state.current_bitmap = if nq.is_multi_select { "0".repeat(nq.options.len()) } else { String::new() };
        state.waiting_for_write_in = false;
        let show_prev = state.current_index > 0;
        cli.sessions.set_ask_state(sid, state.clone()).await;
        crate::messenger::web::events_bridge::tg_ask_by_sid(chat_id.0, sid, false, "").await;
        let markup = super::ask_helpers::build_ask_keyboard_helper(sid, nq, &state.current_bitmap, show_prev);
        let html = super::formatting::markdown_to_telegram_html(&nq.question);
        let _ = bot.edit_message_text(chat_id, msg_id, html).parse_mode(ParseMode::Html).reply_markup(markup).await;
        Ok(true)
    } else {
        cli.sessions.set_ask_state(sid, state.clone()).await;
        cli.sessions.set_ask_active(sid, false).await;
        let sum = state.answers.last().map(String::as_str).unwrap_or("");
        crate::messenger::web::events_bridge::tg_ask_by_sid(chat_id.0, sid, true, sum).await;
        Ok(false)
    }
}

pub(crate) async fn process_answer(
    bot: &Bot,
    msg: &Message,
    sid: &str,
    choice: (usize, &str),
    cli: &AntigravityCli,
    sess_ctx: (&std::sync::Arc<SessionManager>, SessionData),
    config: &CliConfig,
) -> Result<(), teloxide::RequestError> {
    let (idx, data) = choice;
    let (sessions, sess) = sess_ctx;
    if let Some(mut state) = cli.sessions.get_ask_state(sid).await {
        let chosen = state.questions.get(state.current_index)
            .and_then(|q| q.options.get(idx)).cloned()
            .unwrap_or_else(|| format!("Option {}", idx + 1));
        super::history::log_telegram_message(
            &config.working_dir, sid, (sess.topic_id, sess.topic_name.as_deref()),
            "user", Some(msg.id.0), &format!("Selected Option: {}", chosen), (true, None),
        );
        clear_previous_write_in_if_any(sid, cli, &state, true).await;
        let _ = cli.sessions.write_to_session(sid, &format!("{}", idx + 1)).await;
        if state.current_index < state.answers.len() {
            state.answers[state.current_index] = chosen;
        }
        let turn_guard = cli.begin_topic_turn(msg.chat.id.0, sess.topic_id.or_else(|| super::get_topic_id(msg)));
        if !advance_ask_index_or_finish(bot, msg.chat.id, msg.id, sid, cli, &mut state).await? {
            submit_peer_ask_answers_if_needed(msg.chat.id.0, &sess, config, &state.answers).await;
            super::ask_helpers::update_telegram_ask_ui(bot, msg, idx, data).await;
            spawn_cli_stream_in_background(
                bot, msg, sid.to_string(), cli.clone(), (sessions.clone(), sess), config.clone(), turn_guard,
            );
        }
    }
    Ok(())
}

pub(crate) async fn process_submit(
    bot: &Bot,
    msg: &Message,
    sid: &str,
    submission: (&str, &[String]),
    cli: &AntigravityCli,
    sess_ctx: (&std::sync::Arc<SessionManager>, SessionData),
    config: &CliConfig,
) -> Result<(), teloxide::RequestError> {
    let (ks, opts) = submission;
    let (sessions, sess) = sess_ctx;
    if let Some(mut state) = cli.sessions.get_ask_state(sid).await {
        super::history::log_telegram_message(
            &config.working_dir, sid, (sess.topic_id, sess.topic_name.as_deref()),
            "user", Some(msg.id.0), &format!("Submitted Multi-select: [{}]", opts.join(", ")), (true, None),
        );
        let has_wi = opts.iter().any(|o| o.to_lowercase().contains("write-in") || o.contains("직접 입력"));
        clear_previous_write_in_if_any(sid, cli, &state, !has_wi).await;
        let _ = cli.sessions.write_to_session(sid, ks).await;
        if state.current_index < state.answers.len() {
            state.answers[state.current_index] = opts.join(", ");
        }
        let turn_guard = cli.begin_topic_turn(msg.chat.id.0, sess.topic_id.or_else(|| super::get_topic_id(msg)));
        if !advance_ask_index_or_finish(bot, msg.chat.id, msg.id, sid, cli, &mut state).await? {
            submit_peer_ask_answers_if_needed(msg.chat.id.0, &sess, config, &state.answers).await;
            let ch = if opts.is_empty() { "None".to_string() } else { opts.join(", ") };
            let txt = format!("{}\n\n(Selected: **{}**)", msg.text().unwrap_or(""), ch);
            let html = super::formatting::markdown_to_telegram_html(&txt);
            let _ = bot.edit_message_text(msg.chat.id, msg.id, html).parse_mode(ParseMode::Html).await;
            spawn_cli_stream_in_background(
                bot, msg, sid.to_string(), cli.clone(), (sessions.clone(), sess), config.clone(), turn_guard,
            );
        }
    }
    Ok(())
}

fn log_ask_question_result(
    config: &CliConfig,
    sess_id: &str,
    topic: (Option<i64>, Option<&str>),
    msg_id: Option<i32>,
    ask: &AskQuestionData,
    status: (bool, Option<&str>),
) {
    let options_str = ask.options.join(", ");
    let text_to_log = format!("Question: {}\nOptions: [{}]", ask.question, options_str);
    super::history::log_telegram_message(
        &config.working_dir,
        sess_id,
        topic,
        "bot",
        msg_id,
        &text_to_log,
        status,
    );
}

pub(crate) async fn handle_stream_ask_question(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    ask: AskQuestionData,
    session_data: &SessionData,
    config: &CliConfig,
) -> Result<i32, teloxide::RequestError> {
    let prov = SessionData::effective_provider(&session_data.provider, &config.provider);
    let sess_id = session_data.get_session_id(prov);
    let markup = if ask.is_multi_select {
        let initial_bitmap = "0".repeat(ask.options.len());
        build_multi_select_keyboard(&sess_id, &ask.options, &initial_bitmap, false)
    } else {
        build_single_select_keyboard(&sess_id, &ask.options, false)
    };
    let html_question = super::formatting::markdown_to_telegram_html(&ask.question);
    let mut req = bot.send_message(chat_id, html_question)
        .parse_mode(ParseMode::Html)
        .reply_markup(markup);
    if let Some(tid) = thread_id {
        req = req.message_thread_id(ThreadId(MessageId(tid)));
    }
    let res = req.await;
    let (sent_id, ok, err) = match &res {
        Ok(s) => (Some(s.id.0), true, None),
        Err(e) => (None, false, Some(e.to_string())),
    };
    log_ask_question_result(
        config,
        &sess_id,
        (session_data.topic_id, session_data.topic_name.as_deref()),
        sent_id,
        &ask,
        (ok, err.as_deref()),
    );
    res.map(|s| s.id.0)
}

pub(crate) use super::commands_provider::{is_stale_ask_session, register_and_verify_ask_state};

pub(crate) async fn handle_ask_question_event(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    ask: Vec<AskQuestionData>,
    session_data: &SessionData,
    ctx: (&CliConfig, &AntigravityCli, &SessionManager),
) -> Result<(), teloxide::RequestError> {
    let (config, cli, sessions) = ctx;
    if !ask.is_empty() {
        let prov = SessionData::effective_provider(&session_data.provider, &config.provider);
        let s0 = session_data.get_session_id(prov);
        if is_stale_ask_session(sessions, config, session_data, prov, &s0).await {
            super::commands_provider::cancel_abandoned_provider_ask(cli, prov, chat_id.0, session_data.topic_id, &s0).await;
            return Ok(());
        }
        let eff = super::message_session::sync_live_peer_ask_session(sessions, config, session_data, prov, chat_id.0).await;
        let sid = eff.get_session_id(prov);
        if is_stale_ask_session(sessions, config, &eff, prov, &sid).await {
            super::commands_provider::cancel_abandoned_provider_ask(cli, prov, chat_id.0, eff.topic_id, &sid).await;
            return Ok(());
        }
        if let Ok(mid) = handle_stream_ask_question(bot, chat_id, thread_id, ask[0].clone(), &eff, config).await {
            register_and_verify_ask_state(bot, chat_id, mid, ask, &eff, ctx).await;
        } else {
            super::commands_provider::cancel_abandoned_provider_ask(cli, prov, chat_id.0, eff.topic_id, &sid).await;
        }
    }
    Ok(())
}
