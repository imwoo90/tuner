//! # Telegram Keyboard Multi-Select Helper Module
//!
//! This module provides utilities to build multi-select and single-select inline keyboards,
//! parse multi-select callback data, and translate selections into terminal keystrokes.

use teloxide::prelude::*;

pub(crate) fn build_single_select_keyboard(
    sess_id: &str,
    options: &[String],
    show_prev: bool,
) -> teloxide::types::InlineKeyboardMarkup {
    let mut keyboard = Vec::new();
    for (i, opt) in options.iter().enumerate() {
        let lower = opt.to_lowercase();
        if lower.contains("write-in") || lower.contains("직접 입력") {
            continue;
        }
        let callback_data = format!("ask_ans:{}:{}", sess_id, i);
        keyboard.push(vec![teloxide::types::InlineKeyboardButton::callback(opt, callback_data)]);
    }
    if show_prev {
        keyboard.push(vec![teloxide::types::InlineKeyboardButton::callback(
            "⬅️ 이전 질문 (Prev)",
            format!("ask_prev:{}", sess_id),
        )]);
    }
    teloxide::types::InlineKeyboardMarkup::new(keyboard)
}

pub(crate) fn get_bitmap_for_question(
    q: &crate::cli::AskQuestionData,
    answers: &[String],
    index: usize,
) -> String {
    if !q.is_multi_select {
        return String::new();
    }
    if let Some(ans) = answers.get(index) {
        if ans.is_empty() {
            return "0".repeat(q.options.len());
        }
        let parts: Vec<&str> = ans.split(", ").map(|s| s.trim()).collect();
        let mut bm = String::new();
        for opt in &q.options {
            if parts.contains(&opt.as_str()) {
                bm.push('1');
            } else {
                bm.push('0');
            }
        }
        bm
    } else {
        "0".repeat(q.options.len())
    }
}

pub(crate) fn build_multi_select_keyboard(
    sess_id: &str,
    options: &[String],
    bitmap: &str,
    show_prev: bool,
) -> teloxide::types::InlineKeyboardMarkup {
    let mut keyboard = Vec::new();
    for (i, opt) in options.iter().enumerate() {
        let lower = opt.to_lowercase();
        if lower.contains("write-in") || lower.contains("직접 입력") {
            continue;
        }
        let is_checked = bitmap.chars().nth(i).unwrap_or('0') == '1';
        let prefix = if is_checked { "✅ " } else { "⬜ " };
        let button_text = format!("{}{}", prefix, opt);
        let callback_data = format!("ask_mul:{}:{}:{}", sess_id, i, bitmap);
        keyboard.push(vec![teloxide::types::InlineKeyboardButton::callback(button_text, callback_data)]);
    }
    keyboard.push(vec![
        teloxide::types::InlineKeyboardButton::callback(
            "완료 (Submit)",
            format!("ask_sub:{}:{}", sess_id, bitmap),
        ),
    ]);
    if show_prev {
        keyboard.push(vec![teloxide::types::InlineKeyboardButton::callback(
            "⬅️ 이전 질문 (Prev)",
            format!("ask_prev:{}", sess_id),
        )]);
    }
    teloxide::types::InlineKeyboardMarkup::new(keyboard)
}

pub(crate) async fn handle_ask_multi_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    data: &str,
    sessions: &crate::cli::antigravity::session::SessionManager,
) {
    let Some(sid) = data.split(':').nth(1) else { return; };
    let Some(idx) = data.split(':').nth(2).and_then(|i| i.parse::<usize>().ok()) else { return; };
    let Some(bitmap) = data.split(':').nth(3) else { return; };
    if let Some(mut state) = sessions.get_ask_state(sid).await {
        let q = &state.questions[state.current_index];
        if let Some(opt) = q.options.get(idx) {
            if opt.to_lowercase().contains("write-in") || opt.contains("직접 입력") {
                state.waiting_for_write_in = true;
                sessions.set_ask_state(sid, state).await;
                let txt = format!("{}\n\n✍️ **[직접 입력 대기 중...]**\n채팅창에 직접 원하는 답변을 입력한 후 전송해 주세요.", msg.text().unwrap_or(""));
                let html = super::formatting::markdown_to_telegram_html(&txt);
                let _ = bot.edit_message_text(msg.chat.id, msg.id, html).parse_mode(teloxide::types::ParseMode::Html).await;
                return;
            }
        }
        let nb: String = bitmap.chars().enumerate().map(|(i, c)| if i == idx { if c == '1' { '0' } else { '1' } } else { c }).collect();
        state.current_bitmap = nb.clone();
        sessions.set_ask_state(sid, state.clone()).await;
        let markup = build_multi_select_keyboard(sid, &q.options, &nb, state.current_index > 0);
        let _ = bot.edit_message_reply_markup(msg.chat.id, msg.id).reply_markup(markup).await;
    }
}

pub(crate) fn extract_options_from_markup(reply_markup: &teloxide::types::InlineKeyboardMarkup) -> Vec<String> {
    let mut options = Vec::new();
    for row in &reply_markup.inline_keyboard {
        for button in row {
            if let teloxide::types::InlineKeyboardButtonKind::CallbackData(cbd) = &button.kind {
                if cbd.starts_with("ask_mul:") {
                    let opt_name = button.text.trim_start_matches("✅ ").trim_start_matches("⬜ ").to_string();
                    options.push(opt_name);
                }
            }
        }
    }
    options
}

fn build_prev_bitmap(prev_ans: &str, options: &[String]) -> String {
    if prev_ans.is_empty() {
        return "0".repeat(options.len());
    }
    let parts: Vec<&str> = prev_ans.split(", ").map(|s| s.trim()).collect();
    let mut bm = String::new();
    for opt in options {
        if parts.contains(&opt.as_str()) {
            bm.push('1');
        } else {
            bm.push('0');
        }
    }
    bm
}

pub(crate) fn get_multiselect_keystrokes_and_options(
    reply_markup: &teloxide::types::InlineKeyboardMarkup,
    bitmap: &str,
    prev_ans: &str,
    full_options: &[String],
) -> (String, Vec<String>) {
    let mut selected = Vec::new();
    let mut keystrokes = String::new();
    let mut checked_indices = Vec::new();

    let prev_bitmap = build_prev_bitmap(prev_ans, full_options);

    for row in &reply_markup.inline_keyboard {
        for button in row {
            if let teloxide::types::InlineKeyboardButtonKind::CallbackData(cbd) = &button.kind {
                if cbd.starts_with("ask_mul:") {
                    if let Some(opt_index) = cbd.split(':').nth(2).and_then(|s| s.parse::<usize>().ok()) {
                        let is_checked = bitmap.chars().nth(opt_index).unwrap_or('0') == '1';
                        let was_checked = prev_bitmap.chars().nth(opt_index).unwrap_or('0') == '1';
                        let opt_name = button.text.trim_start_matches("✅ ").trim_start_matches("⬜ ").to_string();
                        if is_checked {
                            selected.push(opt_name);
                        }
                        if is_checked != was_checked {
                            checked_indices.push(opt_index);
                        }
                    }
                }
            }
        }
    }

    checked_indices.sort_unstable();

    let mut last_idx = 0;
    for &idx in &checked_indices {
        let diff = idx - last_idx;
        keystrokes.push_str(&"j".repeat(diff));
        keystrokes.push(' ');
        last_idx = idx;
    }
    keystrokes.push('\r');
    (keystrokes, selected)
}
