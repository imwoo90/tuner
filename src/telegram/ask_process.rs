//! # Interactive Ask and Question Handler Process
//!
//! ## Overview
//! Orchestrates the interactive answer submission flow for agent prompts (`ask_question`).
//! Feeds responses back to background execution PTY streams and handles state transitions.
//!
//! ## Collaboration Graph
//! - Interacts with [`super::ask_helpers`] to submit input.
//! - Used by [`super::handler::handle_message`] to route new messages.
//!
//! ## Search Tags
//! #ask-process, #user-input, #pty-feedback

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::session::manager::SessionManager;
use super::multi_select::{build_multi_select_keyboard, build_single_select_keyboard};
use super::ask_helpers::find_write_in_index;



pub(crate) fn spawn_cli_stream_in_background(
    bot: &teloxide::Bot,
    msg: &Message,
    keystrokes: String,
    session_id: String,
    cli: AntigravityCli,
    sessions: std::sync::Arc<SessionManager>,
    sess: crate::session::data::SessionData,
    config: CliConfig,
) {
    let bot_clone = bot.clone();
    let msg_clone = msg.clone();
    tokio::spawn(async move {
        let _ = super::run_cli_stream(&bot_clone, &msg_clone, &keystrokes, &session_id, &cli, &sessions, sess, &config).await;
    });
}

pub(crate) async fn clear_previous_write_in_if_any(
    sid: &str,
    cli: &AntigravityCli,
    state: &crate::cli::antigravity::session::AskState,
    should_clear: bool,
) {
    if !should_clear {
        return;
    }
    let prev_text = state.answers.get(state.current_index).cloned().unwrap_or_default();
    if !prev_text.is_empty() {
        if let Some(q) = state.questions.get(state.current_index) {
            let w_idx = find_write_in_index(&q.options);
            if w_idx <= q.options.len() {
                // 1. Select Write-in
                let _ = cli.sessions.write_to_session(sid, &format!("{}", w_idx + 1)).await;
                tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
                // 2. Clear text and submit empty
                let backspaces = "\x7F".repeat(prev_text.len());
                let _ = cli.sessions.write_to_session(sid, &format!("{}\r", backspaces)).await;
                tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
                // 3. Backtrack
                let _ = cli.sessions.write_to_session(sid, "\x1B[D").await;
                tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
            }
        }
    }
}

pub(crate) async fn advance_ask_index_or_finish(
    bot: &teloxide::Bot,
    chat_id: teloxide::types::ChatId,
    msg_id: teloxide::types::MessageId,
    sid: &str,
    cli: &AntigravityCli,
    state: &mut crate::cli::antigravity::session::AskState,
) -> Result<bool, teloxide::RequestError> {
    if state.current_index + 1 < state.questions.len() {
        state.current_index += 1;
        let nq = &state.questions[state.current_index];
        state.current_bitmap = nq.is_multi_select.then(|| "0".repeat(nq.options.len())).unwrap_or_default();
        state.waiting_for_write_in = false;
        let show_prev = state.current_index > 0;
        cli.sessions.set_ask_state(sid, state.clone()).await;
        let markup = super::ask_helpers::build_ask_keyboard_helper(sid, nq, &state.current_bitmap, show_prev);
        let html = super::formatting::markdown_to_telegram_html(&nq.question);
        let _ = bot.edit_message_text(chat_id, msg_id, html)
            .parse_mode(teloxide::types::ParseMode::Html)
            .reply_markup(markup)
            .await;
        Ok(true)
    } else {
        cli.sessions.set_ask_state(sid, state.clone()).await;
        cli.sessions.set_ask_active(sid, false).await;
        Ok(false)
    }
}

pub(crate) async fn process_answer(
    bot: &teloxide::Bot,
    msg: &Message,
    sid: &str,
    idx: usize,
    data: &str,
    cli: &AntigravityCli,
    sessions: &std::sync::Arc<SessionManager>,
    sess: crate::session::data::SessionData,
    config: &CliConfig,
) -> Result<(), teloxide::RequestError> {
    if let Some(mut state) = cli.sessions.get_ask_state(sid).await {
        let mut chosen = format!("Option {}", idx + 1);
        if let Some(current_q) = state.questions.get(state.current_index) {
            if let Some(opt) = current_q.options.get(idx) {
                chosen = opt.clone();
            }
        }
        let user_action_text = format!("Selected Option: {}", chosen);
        super::history::log_telegram_message(
            &config.working_dir,
            sid,
            "user",
            Some(msg.id.0),
            &user_action_text,
            true,
            None,
        );

        clear_previous_write_in_if_any(sid, cli, &state, true).await;
        let _ = cli.sessions.write_to_session(sid, &format!("{}", idx + 1)).await;

        if state.current_index < state.answers.len() {
            state.answers[state.current_index] = String::new();
        }

        if !advance_ask_index_or_finish(bot, msg.chat.id, msg.id, sid, cli, &mut state).await? {
            super::ask_helpers::update_telegram_ask_ui(bot, msg, idx, data).await;
            spawn_cli_stream_in_background(
                bot, msg, String::new(), sid.to_string(), cli.clone(), sessions.clone(), sess, config.clone()
            );
        }
    }
    Ok(())
}

pub(crate) async fn process_submit(
    bot: &teloxide::Bot,
    msg: &Message,
    sid: &str,
    ks: &str,
    opts: &[String],
    cli: &AntigravityCli,
    sessions: &std::sync::Arc<SessionManager>,
    sess: crate::session::data::SessionData,
    config: &CliConfig,
) -> Result<(), teloxide::RequestError> {
    if let Some(mut state) = cli.sessions.get_ask_state(sid).await {
        let user_action_text = format!("Submitted Multi-select: [{}]", opts.join(", "));
        super::history::log_telegram_message(
            &config.working_dir,
            sid,
            "user",
            Some(msg.id.0),
            &user_action_text,
            true,
            None,
        );

        let contains_write_in = opts.iter().any(|opt| {
            opt.to_lowercase().contains("write-in") || opt.contains("직접 입력")
        });
        clear_previous_write_in_if_any(sid, cli, &state, !contains_write_in).await;
        let _ = cli.sessions.write_to_session(sid, ks).await;

        if state.current_index < state.answers.len() {
            state.answers[state.current_index] = if opts.is_empty() {
                String::new()
            } else {
                opts.join(", ")
            };
        }

        if !advance_ask_index_or_finish(bot, msg.chat.id, msg.id, sid, cli, &mut state).await? {
            let ch = if opts.is_empty() { "None".to_string() } else { opts.join(", ") };
            let txt = format!("{}\n\n(Selected: **{}**)", msg.text().unwrap_or(""), ch);
            let html = super::formatting::markdown_to_telegram_html(&txt);
            let _ = bot.edit_message_text(msg.chat.id, msg.id, html).parse_mode(teloxide::types::ParseMode::Html).await;
            spawn_cli_stream_in_background(
                bot, msg, String::new(), sid.to_string(), cli.clone(), sessions.clone(), sess, config.clone()
            );
        }
    }
    Ok(())
}

fn log_ask_question_result(
    config: &CliConfig,
    sess_id: &str,
    msg_id: Option<i32>,
    ask: &crate::cli::AskQuestionData,
    is_success: bool,
    error: Option<&str>,
) {
    let options_str = ask.options.join(", ");
    let text_to_log = format!("Question: {}\nOptions: [{}]", ask.question, options_str);
    super::history::log_telegram_message(
        &config.working_dir,
        sess_id,
        "bot",
        msg_id,
        &text_to_log,
        is_success,
        error,
    );
}

pub(crate) async fn handle_stream_ask_question(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    ask: crate::cli::AskQuestionData,
    session_data: &crate::session::data::SessionData,
    config: &CliConfig,
) -> Result<i32, teloxide::RequestError> {
    let sess_id = session_data.get_session_id(&config.provider);
    let markup = if ask.is_multi_select {
        let initial_bitmap = "0".repeat(ask.options.len());
        build_multi_select_keyboard(&sess_id, &ask.options, &initial_bitmap, false)
    } else {
        build_single_select_keyboard(&sess_id, &ask.options, false)
    };
    let html_question = super::formatting::markdown_to_telegram_html(&ask.question);
    let mut req = bot.send_message(chat_id, html_question)
        .parse_mode(teloxide::types::ParseMode::Html)
        .reply_markup(markup);
    if let Some(tid) = thread_id {
        req = req.message_thread_id(tid);
    }
    match req.await {
        Ok(sent) => {
            log_ask_question_result(config, &sess_id, Some(sent.id.0), &ask, true, None);
            Ok(sent.id.0)
        }
        Err(e) => {
            log_ask_question_result(config, &sess_id, None, &ask, false, Some(&e.to_string()));
            Err(e)
        }
    }
}

pub(crate) async fn handle_ask_question_event(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    ask: Vec<crate::cli::AskQuestionData>,
    session_data: &crate::session::data::SessionData,
    config: &CliConfig,
    cli: &crate::cli::antigravity::AntigravityCli,
) -> Result<(), teloxide::RequestError> {
    if !ask.is_empty() {
        let sess_id = session_data.get_session_id(&config.provider);
        let first_question = ask[0].clone();
        if let Ok(msg_id) = handle_stream_ask_question(bot, chat_id, thread_id, first_question.clone(), session_data, config).await {
            let initial_bitmap = if first_question.is_multi_select {
                "0".repeat(first_question.options.len())
            } else {
                String::new()
            };
            let ask_len = ask.len();
            let state = crate::cli::antigravity::session::AskState {
                msg_id,
                questions: ask,
                current_index: 0,
                answers: vec![String::new(); ask_len],
                current_bitmap: initial_bitmap,
                waiting_for_write_in: false,
            };
            cli.sessions.set_ask_state(&sess_id, state).await;
        }
    }
    Ok(())
}
