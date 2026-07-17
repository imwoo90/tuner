use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::session::manager::SessionManager;
use crate::cli::antigravity::session::AskState;
use super::multi_select::get_multiselect_keystrokes_and_options;
use super::ask_process::{
    process_answer, process_submit,
};
use super::ask_helpers::build_ask_keyboard_helper;

async fn handle_write_in_transition(
    bot: &teloxide::Bot,
    msg: &Message,
    sid: &str,
    mut state: AskState,
    opt: &str,
    cli: &AntigravityCli,
    config: &CliConfig,
) {
    state.waiting_for_write_in = true;
    cli.sessions.set_ask_state(sid, state).await;
    super::history::log_telegram_message(
        &config.working_dir,
        sid,
        "user",
        Some(msg.id.0),
        &format!("Selected Option: {} (Waiting for Write-in)", opt),
        true,
        None,
    );
    let txt = format!("{}\n\n✍️ **[직접 입력 대기 중...]**\n채팅창에 직접 원하는 답변을 입력한 후 전송해 주세요.", msg.text().unwrap_or(""));
    let html = super::formatting::markdown_to_telegram_html(&txt);
    let _ = bot.edit_message_text(msg.chat.id, msg.id, html).parse_mode(teloxide::types::ParseMode::Html).await;
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
    
    if let Some(state) = cli.sessions.get_ask_state(sid).await {
        if let Some(current_q) = state.questions.get(state.current_index) {
            if let Some(opt) = current_q.options.get(idx) {
                let is_write_in = opt.to_lowercase().contains("write-in") || opt.contains("직접 입력");
                if is_write_in {
                    let opt_clone = opt.clone();
                    handle_write_in_transition(bot, msg, sid, state.clone(), &opt_clone, cli, config).await;
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
            super::history::log_telegram_message(
                &_c.working_dir,
                sid,
                "user",
                Some(msg.id.0),
                "Clicked [Prev] Button",
                true,
                None,
            );
            let key = "\x1B[D";
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
