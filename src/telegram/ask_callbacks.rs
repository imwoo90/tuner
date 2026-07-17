use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::session::manager::SessionManager;
use super::multi_select::{
    build_multi_select_keyboard, build_single_select_keyboard,
    get_multiselect_keystrokes_and_options
};

pub(crate) fn build_ask_keyboard_helper(sid: &str, nq: &crate::cli::AskQuestionData, bitmap: &str, show_prev: bool) -> teloxide::types::InlineKeyboardMarkup {
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
                if let teloxide::types::InlineKeyboardButtonKind::CallbackData(cbd) = &b.kind {
                    if cbd == data { chosen = b.text.clone(); }
                }
            }
        }
    }
    let txt = format!("{}\n\n(Selected: **{}**)", msg.text().unwrap_or(""), chosen);
    let html = super::formatting::markdown_to_telegram_html(&txt);
    let _ = bot.edit_message_text(msg.chat.id, msg.id, html).parse_mode(teloxide::types::ParseMode::Html).await;
}

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

async fn process_answer(
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
        let _ = cli.sessions.write_to_session(sid, &format!("{}", idx + 1)).await;
        if state.current_index + 1 < state.questions.len() {
            state.current_index += 1;
            let nq = &state.questions[state.current_index];
            state.current_bitmap = nq.is_multi_select.then(|| "0".repeat(nq.options.len())).unwrap_or_default();
            state.waiting_for_write_in = false;
            let show_prev = state.current_index > 0;
            cli.sessions.set_ask_state(sid, state.clone()).await;
            let markup = build_ask_keyboard_helper(sid, nq, &state.current_bitmap, show_prev);
            let html = super::formatting::markdown_to_telegram_html(&nq.question);
            let _ = bot.edit_message_text(msg.chat.id, msg.id, html).parse_mode(teloxide::types::ParseMode::Html).reply_markup(markup).await;
        } else {
            cli.sessions.set_ask_active(sid, false).await;
            update_telegram_ask_ui(bot, msg, idx, data).await;
            spawn_cli_stream_in_background(bot, msg, String::new(), sid.to_string(), cli.clone(), sessions.clone(), sess, config.clone());
        }
    }
    Ok(())
}

pub(crate) async fn handle_ask_answer_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    data: &str,
    cli: &AntigravityCli,
    sessions: &std::sync::Arc<SessionManager>,
    config: &CliConfig,
) {
    let Some(sid) = data.split(':').nth(1) else { return; };
    let Some(idx) = data.split(':').nth(2).and_then(|i| i.parse::<usize>().ok()) else { return; };
    
    if let Some(mut state) = cli.sessions.get_ask_state(sid).await {
        if let Some(current_q) = state.questions.get(state.current_index) {
            if let Some(opt) = current_q.options.get(idx) {
                let is_write_in = opt.to_lowercase().contains("write-in") || opt.contains("직접 입력");
                if is_write_in {
                    state.waiting_for_write_in = true;
                    cli.sessions.set_ask_state(sid, state).await;
                    let txt = format!("{}\n\n✍️ **[직접 입력 대기 중...]**\n채팅창에 직접 원하는 답변을 입력한 후 전송해 주세요.", msg.text().unwrap_or(""));
                    let html = super::formatting::markdown_to_telegram_html(&txt);
                    let _ = bot.edit_message_text(msg.chat.id, msg.id, html).parse_mode(teloxide::types::ParseMode::Html).await;
                    return;
                }
            }
        }
    }

    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let dm = config.model.as_deref().unwrap_or("antigravity-default");
    if let Ok((sess, _)) = sessions.resolve_session(&key, &config.provider, dm).await {
        let _ = process_answer(bot, msg, sid, idx, data, cli, sessions, sess, config).await;
    }
}

async fn process_submit(
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
        let _ = cli.sessions.write_to_session(sid, ks).await;
        if state.current_index + 1 < state.questions.len() {
            state.current_index += 1;
            let nq = &state.questions[state.current_index];
            state.current_bitmap = nq.is_multi_select.then(|| "0".repeat(nq.options.len())).unwrap_or_default();
            state.waiting_for_write_in = false;
            let show_prev = state.current_index > 0;
            cli.sessions.set_ask_state(sid, state.clone()).await;
            let markup = build_ask_keyboard_helper(sid, nq, &state.current_bitmap, show_prev);
            let html = super::formatting::markdown_to_telegram_html(&nq.question);
            let _ = bot.edit_message_text(msg.chat.id, msg.id, html).parse_mode(teloxide::types::ParseMode::Html).reply_markup(markup).await;
        } else {
            cli.sessions.set_ask_active(sid, false).await;
            let ch = if opts.is_empty() { "None".to_string() } else { opts.join(", ") };
            let txt = format!("{}\n\n(Selected: **{}**)", msg.text().unwrap_or(""), ch);
            let html = super::formatting::markdown_to_telegram_html(&txt);
            let _ = bot.edit_message_text(msg.chat.id, msg.id, html).parse_mode(teloxide::types::ParseMode::Html).await;
            spawn_cli_stream_in_background(bot, msg, String::new(), sid.to_string(), cli.clone(), sessions.clone(), sess, config.clone());
        }
    }
    Ok(())
}

pub(crate) async fn handle_ask_submit_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    data: &str,
    cli: &AntigravityCli,
    sessions: &std::sync::Arc<SessionManager>,
    config: &CliConfig,
) {
    let Some(sid) = data.split(':').nth(1) else { return; };
    let Some(bitmap) = data.split(':').nth(2) else { return; };
    let mut ks = "\r".to_string();
    let mut opts = Vec::new();
    if let Some(rm) = msg.reply_markup() {
        let (k, o) = get_multiselect_keystrokes_and_options(rm, bitmap);
        ks = k;
        opts = o;
    }
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let dm = config.model.as_deref().unwrap_or("antigravity-default");
    if let Ok((sess, _)) = sessions.resolve_session(&key, &config.provider, dm).await {
        let _ = process_submit(bot, msg, sid, &ks, &opts, cli, sessions, sess, config).await;
    }
}

pub(crate) async fn handle_ask_write_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    data: &str,
    cli: &AntigravityCli,
) {
    let Some(sid) = data.split(':').nth(1) else { return; };
    if let Some(mut state) = cli.sessions.get_ask_state(sid).await {
        state.waiting_for_write_in = true;
        cli.sessions.set_ask_state(sid, state).await;
        let txt = format!("{}\n\n✍️ **[직접 입력 대기 중...]**\n채팅창에 직접 원하는 답변을 입력한 후 전송해 주세요.", msg.text().unwrap_or(""));
        let html = super::formatting::markdown_to_telegram_html(&txt);
        let _ = bot.edit_message_text(msg.chat.id, msg.id, html).parse_mode(teloxide::types::ParseMode::Html).await;
    }
}

pub(crate) async fn handle_ask_prev_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    data: &str,
    cli: &AntigravityCli,
    _s: &std::sync::Arc<SessionManager>,
    _c: &CliConfig,
) {
    let Some(sid) = data.split(':').nth(1) else { return; };
    if let Some(mut state) = cli.sessions.get_ask_state(sid).await {
        if state.current_index > 0 {
            let key = if state.questions[state.current_index].is_multi_select { "\x1B[D" } else { "\x1B" };
            state.current_index -= 1;
            let prev_q = state.questions[state.current_index].clone();
            let bitmap = prev_q.is_multi_select.then(|| "0".repeat(prev_q.options.len())).unwrap_or_default();
            state.current_bitmap = bitmap.clone();
            state.waiting_for_write_in = false;
            let show_prev = state.current_index > 0;
            cli.sessions.set_ask_state(sid, state).await;
            let _ = cli.sessions.write_to_session(sid, key).await;
            let markup = build_ask_keyboard_helper(sid, &prev_q, &bitmap, show_prev);
            let html = super::formatting::markdown_to_telegram_html(&prev_q.question);
            let _ = bot.edit_message_text(msg.chat.id, msg.id, html)
                .parse_mode(teloxide::types::ParseMode::Html)
                .reply_markup(markup)
                .await;
        }
    }
}
