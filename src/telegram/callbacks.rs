//! # Telegram Callback Query Router and Event Handler
//!
//! ## Overview
//! Handles user interaction callbacks triggered from inline keyboard buttons, routing
//! confirmation clicks, option selections, and cancellation events.
//!
//! ## Collaboration Graph
//! - Receives events from Teloxide bot loop.
//! - Feeds selection choices to [`super::ask_callbacks`].
//!
//! ## Search Tags
//! #callback-router, #inline-keyboards, #button-clicks

use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::session::manager::SessionManager;
use crate::cron::manager::CronManager;
use crate::cli::antigravity::AntigravityCli;
use super::ask_callbacks::{
    handle_ask_answer_callback, handle_ask_submit_callback,
    handle_ask_write_callback, handle_ask_prev_callback,
    handle_ask_skip_callback
};
use super::multi_select::handle_ask_multi_callback;

async fn handle_model_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    model: &str,
    sessions: &SessionManager,
    config: &CliConfig,
) {
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let dm = config.model.as_deref().unwrap_or("antigravity-default");
    if let Ok((mut s, _)) = sessions.resolve_session(&key, &config.provider, dm).await {
        s.model = model.to_string();
        let _ = sessions.update_session(&s, 0.0, 0).await;
        let _ = bot.edit_message_text(msg.chat.id, msg.id, crate::t!("bot.model_switch_success", model = model)).await;
    }
}

async fn handle_lang_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    lang: &str,
    sessions: &SessionManager,
    config: &CliConfig,
) {
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let dm = config.model.as_deref().unwrap_or("antigravity-default");
    if let Ok((mut s, _)) = sessions.resolve_session(&key, &config.provider, dm).await {
        s.language = Some(lang.to_string());
        let _ = sessions.update_session(&s, 0.0, 0).await;
        crate::i18n::set_language(lang);
        let _ = bot.edit_message_text(msg.chat.id, msg.id, crate::t!("bot.language_switch_success", language = lang)).await;
    }
}

async fn handle_callback_query_inner(
    bot: teloxide::Bot,
    q: teloxide::types::CallbackQuery,
    config: std::sync::Arc<CliConfig>,
    sessions: std::sync::Arc<SessionManager>,
    cron: std::sync::Arc<CronManager>,
    cli: std::sync::Arc<AntigravityCli>,
) -> Result<(), teloxide::RequestError> {
    use teloxide::prelude::*;
    if let Some(ref d) = q.data {
        if let Some(ref msg) = q.message {
            if let Some(m) = d.strip_prefix("model:") {
                handle_model_callback(&bot, msg, m, &sessions, &config).await;
            } else if let Some(m) = d.strip_prefix("lang:") {
                handle_lang_callback(&bot, msg, m, &sessions, &config).await;
            } else if d.starts_with("crn:") {
                let _ = crate::telegram::cron_selector::handle_cron_callback(&bot, msg.chat.id, msg.id, d, &cron).await;
            } else if d.starts_with("ask_ans:") {
                handle_ask_answer_callback(&bot, msg, d, &cli, &sessions, &config).await;
            } else if d.starts_with("ask_mul:") {
                handle_ask_multi_callback(&bot, msg, d, &cli.sessions).await;
            } else if d.starts_with("ask_sub:") {
                handle_ask_submit_callback(&bot, msg, d, &cli, &sessions, &config).await;
            } else if d.starts_with("ask_write:") {
                handle_ask_write_callback(&bot, msg, d, &cli).await;
            } else if d.starts_with("ask_prev:") {
                handle_ask_prev_callback(&bot, msg, d, &cli, &sessions, &config).await;
            } else if d.starts_with("ask_skip:") {
                handle_ask_skip_callback(&bot, msg, d, &cli, &sessions, &config).await;
            } else if d.starts_with("upg:") {
                let _ = handle_upgrade_callback(&bot, msg, d).await;
            }
        }
        let _ = bot.answer_callback_query(q.id).await;
    }
    Ok(())
}

pub(crate) async fn handle_callback_query(
    bot: teloxide::Bot,
    q: teloxide::types::CallbackQuery,
    config: std::sync::Arc<CliConfig>,
    sessions: std::sync::Arc<SessionManager>,
    cron: std::sync::Arc<CronManager>,
    cli: std::sync::Arc<AntigravityCli>,
) -> Result<(), teloxide::RequestError> {
    handle_callback_query_inner(bot, q, config, sessions, cron, cli).await
}

async fn handle_upgrade_changelog(
    bot: &teloxide::Bot,
    msg: &Message,
    tag: &str,
) -> Result<(), teloxide::RequestError> {
    let version = tag.trim_start_matches('v');
    let client = reqwest::Client::builder().user_agent("tuner-updater").build().unwrap();
    let url = format!("https://api.github.com/repos/imwoo90/tuner/releases/tags/{}", tag);
    let mut text = match client.get(&url).send().await {
        Ok(res) => {
            if let Ok(release) = res.json::<crate::upgrade::GithubRelease>().await {
                release.body.unwrap_or_default()
            } else {
                String::new()
            }
        }
        Err(_) => String::new(),
    };
    if text.is_empty() {
        text = crate::t!("upgrade_handler.no_changelog", version = version);
    }
    let header = crate::t!("upgrade_handler.changelog_header", version = version);
    let mut req = bot.send_message(msg.chat.id, format!("{}\n\n{}", header, text));
    if let Some(tid) = msg.thread_id {
        req = req.message_thread_id(tid);
    }
    let _ = req.await;
    Ok(())
}

async fn handle_upgrade_confirm(
    bot: &teloxide::Bot,
    msg: &Message,
    tag: &str,
) -> Result<(), teloxide::RequestError> {
    let version = tag.trim_start_matches('v');
    let progress_text = crate::t!("upgrade_handler.in_progress", version = version);
    let _ = bot.edit_message_text(msg.chat.id, msg.id, progress_text).await;

    match crate::upgrade::get_latest_release().await {
        Ok(release) => {
            if let Some(asset) = release.assets.iter().find(|a| a.name.ends_with(".tar.gz")) {
                match crate::upgrade::perform_upgrade(&asset.browser_download_url).await {
                    Ok(()) => {
                        let success_text = format!(
                            "{}\n{}",
                            crate::t!("upgrade_handler.restarting"),
                            crate::t!("bot.startup_complete")
                        );
                        let _ = bot.edit_message_text(msg.chat.id, msg.id, success_text).await;
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        std::process::exit(42);
                    }
                    Err(e) => {
                        let current = env!("CARGO_PKG_VERSION");
                        let err_text = crate::t!("upgrade_handler.verification_failed", version = current, details = format!("\nError: {}", e));
                        let _ = bot.edit_message_text(msg.chat.id, msg.id, err_text).await;
                    }
                }
            } else {
                let _ = bot.edit_message_text(msg.chat.id, msg.id, "Could not find a valid release asset for Linux.").await;
            }
        }
        Err(e) => {
            let _ = bot.edit_message_text(msg.chat.id, msg.id, format!("Upgrade aborted: failed to fetch asset URL ({})", e)).await;
        }
    }
    Ok(())
}

async fn handle_upgrade_callback(
    bot: &teloxide::Bot,
    msg: &Message,
    data: &str,
) -> Result<(), teloxide::RequestError> {
    if data == "upg:no" {
        let _ = bot.edit_message_text(msg.chat.id, msg.id, crate::t!("upgrade_handler.skipped")).await;
    } else if let Some(tag) = data.strip_prefix("upg:changelog:") {
        let _ = handle_upgrade_changelog(bot, msg, tag).await;
    } else if let Some(tag) = data.strip_prefix("upg:yes:") {
        let _ = handle_upgrade_confirm(bot, msg, tag).await;
    }
    Ok(())
}
