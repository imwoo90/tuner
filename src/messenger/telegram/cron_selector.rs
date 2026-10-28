//! # Cron Task Configuration Selector for Telegram UI
//!
//! ## Overview
//! Renders interactive inline menus to configure, enable, or disable cron tasks directly
//! from chat messages. Translates menu choices into scheduler commands.
//!
//! ## Search Tags
//! #cron-selector, #interactive-menus, #ui-callbacks

use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId};

use crate::config::CliConfig;
use crate::cron::manager::{CronJob, CronManager};
use crate::session::manager::SessionManager;
use crate::t;
use super::cron_formatter::{fingerprint, format_job_and_buttons, format_target_topic};
use super::TopicNameCache;

const PAGE_SIZE: usize = 4;

fn build_keyboard(
    current_page: usize,
    total_pages: usize,
    mut keyboard: Vec<Vec<InlineKeyboardButton>>,
) -> Vec<Vec<InlineKeyboardButton>> {
    let mut nav_row = Vec::new();
    if current_page > 0 {
        nav_row.push(InlineKeyboardButton::callback(t!("bot.cron_prev_page"), format!("crn:p:{}", current_page)));
    }
    nav_row.push(InlineKeyboardButton::callback(t!("bot.cron_refresh"), format!("crn:r:{}", current_page)));
    if current_page < total_pages - 1 {
        nav_row.push(InlineKeyboardButton::callback(t!("bot.cron_next_page"), format!("crn:n:{}", current_page)));
    }
    keyboard.push(nav_row);

    keyboard.push(vec![
        InlineKeyboardButton::callback(t!("bot.cron_all_on"), format!("crn:ao:{}", current_page)),
        InlineKeyboardButton::callback(t!("bot.cron_all_off"), format!("crn:af:{}", current_page)),
    ]);
    keyboard
}

pub(crate) async fn build_cron_page(
    manager: &CronManager,
    page: usize,
    note: Option<&str>,
    topic_cache: Option<&TopicNameCache>,
) -> Result<(String, InlineKeyboardMarkup), String> {
    let jobs = manager.list_jobs().await?;
    if jobs.is_empty() {
        let text = t!("bot.cron_no_jobs");
        return Ok((text, InlineKeyboardMarkup::new(Vec::<Vec<InlineKeyboardButton>>::new())));
    }

    let total_pages = jobs.len().div_ceil(PAGE_SIZE);
    let current_page = page.min(total_pages - 1);
    let start = current_page * PAGE_SIZE;
    let end = (start + PAGE_SIZE).min(jobs.len());
    let page_jobs = &jobs[start..end];

    let mut lines = vec![t!("bot.cron_list_header"), "".to_string()];
    let mut keyboard = Vec::new();
    let default_tz = manager.default_timezone();

    for (idx, job) in page_jobs.iter().enumerate() {
        let number = start + idx + 1;
        let (line, buttons) = format_job_and_buttons(job, number, idx, current_page, &default_tz, topic_cache);
        lines.push(line);
        keyboard.push(buttons);
    }

    let keyboard = build_keyboard(current_page, total_pages, keyboard);

    lines.push("".to_string());
    if let Some(n) = note {
        lines.push(n.to_string());
    }
    lines.push(t!(
        "bot.cron_page_footer",
        current = (current_page + 1).to_string(),
        total = total_pages.to_string()
    ));

    Ok((lines.join("\n"), InlineKeyboardMarkup::new(keyboard)))
}

async fn apply_toggle(
    manager: &CronManager,
    job: &CronJob,
    caller_chat_id: Option<i64>,
    caller_topic_id: Option<i64>,
    topic_cache: Option<&TopicNameCache>,
) -> Result<String, String> {
    let new_state = !job.enabled;
    let mut auto_bound = false;
    if new_state && job.chat_id == 0
        && let Some(cid) = caller_chat_id {
            let _ = manager.update_job_target(&job.id, cid, caller_topic_id).await;
            auto_bound = true;
        }
    let _ = manager.set_enabled(&job.id, new_state).await?;
    let state_str = if new_state { t!("bot.cron_state_enabled") } else { t!("bot.cron_state_disabled") };
    let safe_title = html_escape::encode_safe(&job.title).to_string();
    if auto_bound {
        let mut upd = job.clone();
        if let Some(cid) = caller_chat_id { upd.chat_id = cid; upd.topic_id = caller_topic_id; }
        let target = format_target_topic(&upd, topic_cache);
        Ok(t!("bot.cron_toggle_success_target", title = &safe_title, state = state_str, target = target))
    } else {
        Ok(t!("bot.cron_toggle_success", title = &safe_title, state = state_str))
    }
}

async fn handle_toggle_action(
    manager: &CronManager,
    parts: &[&str],
    page: usize,
    caller_chat_id: Option<i64>,
    caller_topic_id: Option<i64>,
    topic_cache: Option<&TopicNameCache>,
) -> Result<Option<String>, String> {
    if parts.len() < 4 { return Ok(None); }
    let slot: usize = parts[2].parse().unwrap_or(0);
    let fp = parts[3];
    let jobs = manager.list_jobs().await?;
    if let Some(job) = jobs.get(page * PAGE_SIZE + slot) {
        if fingerprint(&job.id) == fp {
            return apply_toggle(manager, job, caller_chat_id, caller_topic_id, topic_cache).await.map(Some);
        }
        return Ok(Some(t!("bot.cron_toggle_mismatch")));
    }
    Ok(None)
}

async fn handle_run_now_action(
    bot: &Bot,
    msg_target: (ChatId, MessageId),
    manager: &Arc<CronManager>,
    parts: &[&str],
    page: usize,
    caller_topic_id: Option<i64>,
    aux: CronCallbackAux<'_>,
) -> Result<Option<String>, String> {
    if parts.len() < 4 { return Ok(None); }
    let slot: usize = parts[2].parse().unwrap_or(0);
    let jobs = manager.list_jobs().await?;
    let Some(job) = jobs.get(page * PAGE_SIZE + slot).cloned() else { return Ok(None); };
    if fingerprint(&job.id) != parts[3] {
        return Ok(Some(t!("bot.cron_toggle_mismatch")));
    }
    let safe_title = html_escape::encode_safe(&job.title).to_string();
    let note = t!("bot.cron_run_started", title = &safe_title);
    let (topic_cache, run_ctx) = aux;
    if let Some((cfg, sess)) = run_ctx {
        let (bot_c, cfg_c, sess_c, mgr_c) = (bot.clone(), (*cfg).clone(), (*sess).clone(), manager.clone());
        let (tc_c, lang) = (topic_cache.cloned(), crate::i18n::get_language());
        tokio::spawn(crate::i18n::TASK_ACTIVE_LANG.scope(lang, async move {
            let bus = Arc::new(crate::bus::bus::MessageBus::with_lock_pool(sess_c.lock_pool.clone()));
            let tgts = if !cfg_c.allowed_group_ids.is_empty() { cfg_c.allowed_group_ids.clone() } else { cfg_c.allowed_user_ids.clone() };
            bus.register_transport(Arc::new(super::transport::TelegramTransport::with_targets(bot_c.clone(), tgts).with_context(cfg_c.clone(), sess_c)));
            let _ = crate::cron::scheduler::CronScheduler::new(cfg_c, mgr_c.clone(), bus).execute_job_now(job, msg_target.0.0, caller_topic_id).await;
            let _ = update_message(&bot_c, msg_target.0, msg_target.1, &mgr_c, page, Some(&t!("bot.cron_refreshed_note")), tc_c.as_ref()).await;
        }));
    }
    Ok(Some(note))
}

pub(crate) type CronCallbackAux<'a> = (Option<&'a TopicNameCache>, Option<(&'a Arc<CliConfig>, &'a Arc<SessionManager>)>);

pub(crate) async fn handle_cron_callback(
    bot: &Bot,
    chat_id: ChatId,
    message_id: MessageId,
    data: &str,
    manager: &Arc<CronManager>,
    caller_topic_id: Option<i64>,
    aux: CronCallbackAux<'_>,
) -> Result<(), String> {
    let (topic_cache, _) = aux;
    let parts: Vec<&str> = data["crn:".len()..].split(':').collect();
    if parts.is_empty() { return Ok(()); }

    let action = parts[0];
    let page: usize = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
    let note = match action {
        "p" => return update_message(bot, chat_id, message_id, manager, page.saturating_sub(1), None, topic_cache).await,
        "n" => return update_message(bot, chat_id, message_id, manager, page + 1, None, topic_cache).await,
        "r" => Some(t!("bot.cron_refreshed_note")),
        "ao" | "af" => {
            let enabled = action == "ao";
            let c = manager.set_all_enabled(enabled).await?.to_string();
            Some(if enabled { t!("bot.cron_all_enabled_note", count = c) } else { t!("bot.cron_all_disabled_note", count = c) })
        }
        "t" => handle_toggle_action(manager, &parts, page, Some(chat_id.0), caller_topic_id, topic_cache).await?,
        "run" => handle_run_now_action(bot, (chat_id, message_id), manager, &parts, page, caller_topic_id, aux).await?,
        _ => return Ok(()),
    };
    update_message(bot, chat_id, message_id, manager, page, note.as_deref(), topic_cache).await
}

async fn update_message(
    bot: &Bot,
    chat_id: ChatId,
    message_id: MessageId,
    manager: &CronManager,
    page: usize,
    note: Option<&str>,
    topic_cache: Option<&TopicNameCache>,
) -> Result<(), String> {
    let (text, markup) = build_cron_page(manager, page, note, topic_cache).await?;
    let _ = bot.edit_message_text(chat_id, message_id, text)
        .parse_mode(teloxide::types::ParseMode::Html)
        .reply_markup(markup)
        .await;
    Ok(())
}
