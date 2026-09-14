//! # Telegram Upgrade Command Handler
//!
//! Implements command handlers allowing users to check, download, and initiate self-upgrades
//! directly via Telegram inline keyboards and chat prompts.

//! 
//! ## Search Tags
//! #upgrade

use teloxide::prelude::*;

async fn send_reply(
    bot: &Bot,
    msg: &Message,
    text: impl Into<String>,
) -> Result<Message, teloxide::RequestError> {
    super::commands::send_reply(bot, msg, text).await
}

fn build_upgrade_keyboard(tag: &str, latest: &str, has_body: bool) -> teloxide::types::InlineKeyboardMarkup {
    let mut keyboard = Vec::new();
    keyboard.push(vec![
        teloxide::types::InlineKeyboardButton::callback(
            crate::t!("upgrade.btn_yes"),
            format!("upg:yes:{}", tag)
        )
    ]);
    if has_body {
        keyboard.push(vec![
            teloxide::types::InlineKeyboardButton::callback(
                crate::t!("upgrade.btn_changelog", version = latest),
                format!("upg:changelog:{}", tag)
            )
        ]);
    }
    keyboard.push(vec![
        teloxide::types::InlineKeyboardButton::callback(
            crate::t!("upgrade.btn_not_now"),
            "upg:no".to_string()
        )
    ]);
    teloxide::types::InlineKeyboardMarkup::new(keyboard)
}

async fn edit_reply(
    bot: &Bot,
    chat_id: ChatId,
    mid: teloxide::types::MessageId,
    text: impl Into<String>,
    markup: Option<teloxide::types::InlineKeyboardMarkup>,
) {
    let limiter = super::rate_limiter::global_chat_rate_limiter();
    let text_str = text.into();
    let res = limiter.execute_rate_limited(chat_id, || {
        let t = text_str.clone();
        let m = markup.clone();
        async move {
            let mut req = bot.edit_message_text(chat_id, mid, t);
            if let Some(m_inner) = m {
                req = req.reply_markup(m_inner);
            }
            req.await
        }
    }).await;
    if let Err(e) = res {
        eprintln!("❌ [tuner] Failed to edit upgrade message: {:?}", e);
    }
}

pub(crate) async fn handle_upgrade_command(
    bot: &Bot,
    msg: &Message,
) -> Result<(), teloxide::RequestError> {
    let processing_msg = send_reply(bot, msg, crate::t!("bot.processing")).await?;

    match crate::upgrade::get_latest_release().await {
        Ok(release) => {
            let current = env!("CARGO_PKG_VERSION");
            let latest = release.tag_name.trim_start_matches('v');
            if crate::upgrade::is_newer_version(current, latest) {
                if release.assets.iter().any(|a| a.name.ends_with(".tar.gz")) {
                    let has_body = release.body.as_ref().map(|b| !b.is_empty()).unwrap_or(false);
                    let markup = build_upgrade_keyboard(&release.tag_name, latest, has_body);
                    let header = crate::t!("upgrade.available_header");
                    let body = crate::t!("upgrade.available_body", current = current, latest = latest);
                    edit_reply(bot, msg.chat.id, processing_msg.id, format!("{}\n\n{}", header, body), Some(markup)).await;
                } else {
                    edit_reply(bot, msg.chat.id, processing_msg.id, "Could not find a valid release asset for Linux.", None).await;
                }
            } else {
                let header = crate::t!("upgrade.up_to_date_header");
                let body = crate::t!("upgrade.up_to_date_body", current = current, latest = latest);
                edit_reply(bot, msg.chat.id, processing_msg.id, format!("{}\n\n{}", header, body), None).await;
            }
        }
        Err(e) => {
            edit_reply(bot, msg.chat.id, processing_msg.id, format!("Failed to check for updates: {}", e), None).await;
        }
    }
    Ok(())
}
