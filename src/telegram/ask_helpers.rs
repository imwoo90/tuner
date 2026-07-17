use teloxide::prelude::*;
use crate::cli::antigravity::AntigravityCli;
use crate::session::manager::SessionManager;
use crate::session::data::SessionData;
use crate::config::CliConfig;
use crate::cli::antigravity::session::AskState;
use std::sync::Arc;

fn find_write_in_index(options: &[String]) -> usize {
    for (i, opt) in options.iter().enumerate() {
        let lower = opt.to_lowercase();
        if lower.contains("write-in") || lower.contains("직접 입력") {
            return i;
        }
    }
    options.len()
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
        let w_idx = find_write_in_index(&q.options);
        if state.waiting_for_write_in {
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
        
        if is_write_in {
            let _ = cli.sessions.write_to_session(sid, &format!("{}", w_idx + 1)).await;
            tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
            ip = format!("{}\r", current_text);
            let _ = cli.sessions.write_to_session(sid, &ip).await;
            if q.is_multi_select {
                tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
                let _ = cli.sessions.write_to_session(sid, "\r").await;
            }
        } else {
            let _ = cli.sessions.write_to_session(sid, &ip).await;
        }
    }
    (ip, is_write_in)
}

async fn update_question_ui(
    bot: &teloxide::Bot,
    chat_id: teloxide::types::ChatId,
    mid: i32,
    q: &crate::cli::AskQuestionData,
    markup: teloxide::types::InlineKeyboardMarkup,
) {
    let html = super::formatting::markdown_to_telegram_html(&q.question);
    let _ = bot.edit_message_text(chat_id, teloxide::types::MessageId(mid), html)
        .parse_mode(teloxide::types::ParseMode::Html)
        .reply_markup(markup)
        .await;
}

async fn advance_ask_state(
    bot: &teloxide::Bot,
    msg: &Message,
    sid: &str,
    msg_id: Option<i32>,
    mut state: AskState,
    cli: &AntigravityCli,
    _is_write_in: bool,
    sessions: &Arc<SessionManager>,
    sess: SessionData,
    config: &CliConfig,
) -> Result<(), teloxide::RequestError> {
    let chat_id = msg.chat.id;
    if state.current_index + 1 < state.questions.len() {
        state.current_index += 1;
        let nq = state.questions[state.current_index].clone();
        let bitmap = nq.is_multi_select.then(|| "0".repeat(nq.options.len())).unwrap_or_default();
        state.current_bitmap = bitmap.clone();
        state.waiting_for_write_in = false;
        cli.sessions.set_ask_state(sid, state).await;
        if let Some(mid) = msg_id {
            let markup = super::ask_callbacks::build_ask_keyboard_helper(sid, &nq, &bitmap, true);
            update_question_ui(bot, chat_id, mid, &nq, markup).await;
        }
    } else {
        cli.sessions.set_ask_active(sid, false).await;
        if let Some(mid) = msg_id {
            let _ = bot.edit_message_reply_markup(chat_id, teloxide::types::MessageId(mid)).await;
        }
        super::ask_callbacks::spawn_cli_stream_in_background(
            bot, msg, String::new(), sid.to_string(), cli.clone(), sessions.clone(), sess, config.clone(),
        );
    }
    Ok(())
}

async fn handle_ask_input(
    bot: &teloxide::Bot,
    msg: &Message,
    session_id: &str,
    current_text: &str,
    cli: &AntigravityCli,
    state: AskState,
    sessions: &Arc<SessionManager>,
    sess: SessionData,
    config: &CliConfig,
) -> Result<String, teloxide::RequestError> {
    let msg_id = Some(state.msg_id);
    let (ip, is_write_in) = determine_and_send_input(session_id, current_text, cli, &state).await;
    
    if let Some(mid) = msg_id {
        if let Some(q) = state.questions.get(state.current_index) {
            let txt = format!("{}\n\n(Selected: **{}**)", q.question, current_text);
            let html = super::formatting::markdown_to_telegram_html(&txt);
            let _ = bot.edit_message_text(msg.chat.id, teloxide::types::MessageId(mid), html)
                .parse_mode(teloxide::types::ParseMode::Html)
                .await;
        }
    }

    advance_ask_state(bot, msg, session_id, msg_id, state, cli, is_write_in, sessions, sess, config).await?;
    Ok(ip)
}

pub(crate) async fn feed_active_session_if_running(
    bot: &teloxide::Bot,
    msg: &Message,
    session_id: &str,
    current_text: &str,
    cli: &AntigravityCli,
    sessions: &Arc<SessionManager>,
    sess: SessionData,
    config: &CliConfig,
) -> Result<bool, teloxide::RequestError> {
    let is_active = cli.sessions.is_active(session_id).await;
    let is_running = cli.sessions.is_running(session_id).await;
    let is_ask = cli.sessions.is_ask_active(session_id).await;

    if is_active && (is_running || is_ask) {
        if is_ask {
            if let Some(state) = cli.sessions.get_ask_state(session_id).await {
                let _ = handle_ask_input(bot, msg, session_id, current_text, cli, state, sessions, sess, config).await?;
                return Ok(true);
            }
        }
        if is_running {
            let input_prompt = format!("{}\r", current_text);
            let _ = cli.sessions.write_to_session(session_id, &input_prompt).await;
            return Ok(true);
        }
    }
    Ok(false)
}
