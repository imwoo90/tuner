use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::StreamEvent;
use std::time::{Instant, Duration};
use super::formatting;

pub(crate) async fn handle_stream_result(
    bot: &Bot,
    chat_id: ChatId,
    msg_id: Option<teloxide::types::MessageId>,
    thread_id: Option<i32>,
    resp: crate::cli::CliResponse,
) -> Result<Option<String>, teloxide::RequestError> {
    let mut last_session_id = None;
    if let Some(ref sid) = resp.session_id {
        last_session_id = Some(sid.clone());
    }
    let raw_text = if resp.is_error {
        let code = resp.returncode.unwrap_or(1);
        let error_msg = if !resp.stderr.is_empty() { &resp.stderr } else { &resp.result };
        crate::cli::antigravity::error_parser::parse_cli_error(error_msg, code)
    } else {
        resp.result
    };

    let html_text = formatting::markdown_to_telegram_html(&raw_text);
    let chunks = formatting::split_html_message(&html_text, 4000);

    let mut current_msg_id = msg_id;
    for (i, chunk) in chunks.iter().enumerate() {
        if let Some(mid) = current_msg_id {
            if i == 0 {
                let _ = bot.edit_message_text(chat_id, mid, chunk)
                    .parse_mode(teloxide::types::ParseMode::Html)
                    .await;
            } else {
                let mut req = bot.send_message(chat_id, chunk)
                    .parse_mode(teloxide::types::ParseMode::Html);
                if let Some(tid) = thread_id {
                    req = req.message_thread_id(tid);
                }
                let _ = req.await;
            }
        } else {
            let mut req = bot.send_message(chat_id, chunk)
                .parse_mode(teloxide::types::ParseMode::Html);
            if let Some(tid) = thread_id {
                req = req.message_thread_id(tid);
            }
            if let Ok(sent) = req.await {
                if i == 0 {
                    current_msg_id = Some(sent.id);
                }
            }
        }
    }
    Ok(last_session_id)
}

pub(crate) async fn consume_stream(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    mut stream: futures::stream::BoxStream<'_, StreamEvent>,
    sessions: &crate::session::manager::SessionManager,
    session_data: crate::session::data::SessionData,
    config: &CliConfig,
) -> Result<(), teloxide::RequestError> {
    let mut last_edit = Instant::now();
    let mut last_session_id = None;
    let mut pub_msg_id = None;
    
    use futures::StreamExt;
    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::TextDelta(delta) => {
                if let Some(mid) = pub_msg_id {
                    if Instant::now().duration_since(last_edit) >= Duration::from_secs(2) {
                        let _ = bot.edit_message_text(chat_id, mid, &delta).await;
                        last_edit = Instant::now();
                    }
                } else {
                    let mut req = bot.send_message(chat_id, &delta);
                    if let Some(tid) = thread_id {
                        req = req.message_thread_id(tid);
                    }
                    if let Ok(sent) = req.await {
                        pub_msg_id = Some(sent.id);
                        last_edit = Instant::now();
                    }
                }
            }
            StreamEvent::Result(resp) => {
                if let Ok(Some(sid)) = handle_stream_result(bot, chat_id, pub_msg_id, thread_id, resp).await {
                    last_session_id = Some(sid);
                }
            }
        }
    }

    if let Some(sid) = last_session_id {
        let mut updated = session_data;
        updated.set_session_id(&config.provider, &sid);
        let _ = sessions.update_session(&updated, 0.0, 0).await;
    }
    Ok(())
}
