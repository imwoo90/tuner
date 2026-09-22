//! # Review Callback Interaction Handler
//!
//! Handles Telegram inline button callbacks for on-demand review session activation,
//! validating 30-day session retention, checking disk file existence, and updating message buttons.

use super::review::global_review_manager;
use super::review_store::check_record_files;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, WebAppInfo};

fn build_viewer_buttons(review_url: &str, is_private: bool, session_id: &str) -> InlineKeyboardMarkup {
    let open_btn = if let Ok(parsed) = review_url.parse() {
        if is_private && review_url.starts_with("https://") {
            InlineKeyboardButton::web_app("🌐 뷰어 열기 (30분 유효)", WebAppInfo { url: parsed })
        } else {
            InlineKeyboardButton::url("🌐 뷰어 열기 (30분 유효)", parsed)
        }
    } else {
        InlineKeyboardButton::callback("🌐 뷰어 열기", format!("review:{}", session_id))
    };

    let refresh_btn = InlineKeyboardButton::callback("🔄 링크 갱신", format!("review:{}", session_id));
    let dl_btn = InlineKeyboardButton::callback("📥 직접 받기", format!("dl_files:{}", session_id));
    InlineKeyboardMarkup::new(vec![vec![open_btn], vec![refresh_btn, dl_btn]])
}

pub async fn handle_review_callback(
    bot: &teloxide::Bot,
    q: &teloxide::types::CallbackQuery,
    msg: &teloxide::types::Message,
    session_id: &str,
) {
    let mgr = global_review_manager();
    let record = match mgr.get_session_record(session_id).await {
        Some(rec) => rec,
        None => {
            let _ = bot
                .answer_callback_query(q.id.clone())
                .text("⚠️ 보관 기간(30일)이 만료되어 열람할 수 없습니다.")
                .show_alert(true)
                .await;
            return;
        }
    };

    let (valid_paths, total) = check_record_files(&record);
    if valid_paths.is_empty() {
        let _ = bot
            .answer_callback_query(q.id.clone())
            .text("⚠️ 참조된 파일이 디스크에서 이미 삭제/이동되어 열람할 수 없습니다.")
            .show_alert(true)
            .await;
        return;
    }

    let token = match mgr.activate_session(&valid_paths).await {
        Some(t) => t,
        None => {
            let _ = bot
                .answer_callback_query(q.id.clone())
                .text("⚠️ 파일 세션을 활성화하지 못했습니다.")
                .show_alert(true)
                .await;
            return;
        }
    };

    let review_url = mgr.get_review_url(&token).await;
    let is_private = msg.chat.id.0 > 0;
    let keyboard = build_viewer_buttons(&review_url, is_private, session_id);

    let _ = bot
        .edit_message_reply_markup(msg.chat.id, msg.id)
        .reply_markup(keyboard)
        .await;

    let toast = format_activation_toast(valid_paths.len(), total);
    let _ = bot.answer_callback_query(q.id.clone()).text(toast).await;
}

fn format_activation_toast(valid: usize, total: usize) -> String {
    if valid < total {
        format!("⚠️ 일부 파일 삭제됨 ({}개 중 {}개 유효). [🌐 뷰어 열기]를 눌러주세요.", total, valid)
    } else {
        "✅ 뷰어 링크가 활성화되었습니다. [🌐 뷰어 열기]를 눌러주세요.".to_string()
    }
}
