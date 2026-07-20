//! # Telegram Upgrade Command Handler
//!
//! Implements command handlers allowing users to check, download, and initiate self-upgrades
//! directly via Telegram inline keyboards and chat prompts.

use teloxide::prelude::*;

async fn send_reply(
    bot: &Bot,
    msg: &Message,
    text: impl Into<String>,
) -> Result<Message, teloxide::RequestError> {
    let mut req = bot.send_message(msg.chat.id, text);
    if let Some(tid) = msg.thread_id {
        req = req.message_thread_id(tid);
    }
    req.await
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

pub(crate) async fn handle_upgrade_command(
    bot: &Bot,
    msg: &Message,
) -> Result<(), teloxide::RequestError> {
    if crate::upgrade::is_dev_install() {
        let _ = send_reply(bot, msg, crate::t!("upgrade.dev_body")).await;
        return Ok(());
    }

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
                    let _ = bot.edit_message_text(msg.chat.id, processing_msg.id, format!("{}\n\n{}", header, body))
                        .reply_markup(markup)
                        .await;
                } else {
                    let _ = bot.edit_message_text(msg.chat.id, processing_msg.id, "Could not find a valid release asset for Linux.").await;
                }
            } else {
                let header = crate::t!("upgrade.up_to_date_header");
                let body = crate::t!("upgrade.up_to_date_body", current = current, latest = latest);
                let _ = bot.edit_message_text(msg.chat.id, processing_msg.id, format!("{}\n\n{}", header, body)).await;
            }
        }
        Err(e) => {
            let _ = bot.edit_message_text(msg.chat.id, processing_msg.id, format!("Failed to check for updates: {}", e)).await;
        }
    }
    Ok(())
}
