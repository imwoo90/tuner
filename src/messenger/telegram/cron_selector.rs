//! # Cron Task Configuration Selector for Telegram UI
//!
//! ## Overview
//! Renders interactive inline menus to configure, enable, or disable cron tasks directly
//! from chat messages. Translates menu choices into scheduler commands.
//!
//! ## Collaboration Graph
//! - Invoked during Telegram command execution.
//! - Modifies jobs held in [`CronManager`].
//!
//! ## Search Tags
//! #cron-selector, #interactive-menus, #ui-callbacks

use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId};

use crate::cron::manager::{CronJob, CronManager};
use crate::t;
use super::cron_formatter::{format_schedule_display, format_target_topic};
use super::TopicNameCache;

const PAGE_SIZE: usize = 4;

fn fingerprint(job_id: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    job_id.hash(&mut hasher);
    format!("{:08x}", hasher.finish())
}

fn format_job_and_button(
    job: &CronJob,
    number: usize,
    idx: usize,
    current_page: usize,
    topic_cache: Option<&TopicNameCache>,
) -> (String, InlineKeyboardButton) {
    let status = if job.enabled {
        t!("bot.cron_active")
    } else {
        t!("bot.cron_inactive")
    };
    let last_run_val = job.last_run_status.as_deref().unwrap_or("");
    let last_run = if last_run_val.is_empty() {
        t!("bot.cron_no_last_run")
    } else {
        last_run_val.to_string()
    };
    let number_str = number.to_string();
    let target = format_target_topic(job, topic_cache);
    let model = job.model.as_deref().unwrap_or("default");
    let schedule = format_schedule_display(&job.schedule, &job.timezone);
    let line = t!(
        "bot.cron_job_line",
        number = number_str,
        title = html_escape::encode_safe(&job.title),
        status = status,
        schedule = schedule,
        target = target,
        model = html_escape::encode_safe(model),
        last_run = html_escape::encode_safe(&last_run)
    );
    let button_text = if job.enabled {
        t!("bot.cron_deactivate_num", number = number_str)
    } else {
        t!("bot.cron_activate_num", number = number_str)
    };
    let fp = fingerprint(&job.id);
    let button = InlineKeyboardButton::callback(
        button_text,
        format!("crn:t:{}:{}:{}", current_page, idx, fp),
    );
    (line, button)
}

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

    let total_pages = (jobs.len() + PAGE_SIZE - 1) / PAGE_SIZE;
    let current_page = page.min(total_pages - 1);
    let start = current_page * PAGE_SIZE;
    let end = (start + PAGE_SIZE).min(jobs.len());
    let page_jobs = &jobs[start..end];

    let mut lines = vec![t!("bot.cron_list_header"), "".to_string()];
    let mut keyboard = Vec::new();

    for (idx, job) in page_jobs.iter().enumerate() {
        let number = start + idx + 1;
        let (line, button) = format_job_and_button(job, number, idx, current_page, topic_cache);
        lines.push(line);
        keyboard.push(vec![button]);
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
    if new_state && job.chat_id == 0 {
        if let Some(cid) = caller_chat_id {
            let _ = manager.update_job_target(&job.id, cid, caller_topic_id).await;
            auto_bound = true;
        }
    }
    let _ = manager.set_enabled(&job.id, new_state).await?;
    let state_str = if new_state { t!("bot.cron_state_enabled") } else { t!("bot.cron_state_disabled") };
    if auto_bound {
        let mut upd = job.clone();
        if let Some(cid) = caller_chat_id { upd.chat_id = cid; upd.topic_id = caller_topic_id; }
        let target = format_target_topic(&upd, topic_cache);
        Ok(t!("bot.cron_toggle_success_target", title = upd.title, state = state_str, target = target))
    } else {
        Ok(t!("bot.cron_toggle_success", title = job.title, state = state_str))
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

pub(crate) async fn handle_cron_callback(
    bot: &Bot,
    chat_id: ChatId,
    message_id: MessageId,
    data: &str,
    manager: &CronManager,
    caller_topic_id: Option<i64>,
    topic_cache: Option<&TopicNameCache>,
) -> Result<(), String> {
    let parts: Vec<&str> = data["crn:".len()..].split(':').collect();
    if parts.is_empty() {
        return Ok(());
    }

    let action = parts[0];
    let page: usize = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
    match action {
        "p" => {
            let next_page = if page > 0 { page - 1 } else { 0 };
            update_message(bot, chat_id, message_id, manager, next_page, None, topic_cache).await?;
        }
        "n" => {
            update_message(bot, chat_id, message_id, manager, page + 1, None, topic_cache).await?;
        }
        "r" => {
            let note = t!("bot.cron_refreshed_note");
            update_message(bot, chat_id, message_id, manager, page, Some(&note), topic_cache).await?;
        }
        "ao" | "af" => {
            let enabled = action == "ao";
            let changed_count = manager.set_all_enabled(enabled).await?;
            let changed_count_str = changed_count.to_string();
            let note = if enabled {
                t!("bot.cron_all_enabled_note", count = changed_count_str)
            } else {
                t!("bot.cron_all_disabled_note", count = changed_count_str)
            };
            update_message(bot, chat_id, message_id, manager, page, Some(&note), topic_cache).await?;
        }
        "t" => {
            let toggle_note = handle_toggle_action(
                manager, &parts, page, Some(chat_id.0), caller_topic_id, topic_cache,
            ).await?;
            update_message(bot, chat_id, message_id, manager, page, toggle_note.as_deref(), topic_cache).await?;
        }
        _ => {}
    }

    Ok(())
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
