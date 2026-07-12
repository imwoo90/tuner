use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId};

use crate::cron::manager::{CronJob, CronManager};
use crate::t;

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
    let line = t!(
        "bot.cron_job_line",
        number = number_str,
        title = job.title,
        status = status,
        schedule = job.schedule,
        last_run = last_run,
        folder = job.task_folder
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
        let (line, button) = format_job_and_button(job, number, idx, current_page);
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

async fn handle_toggle_action(
    manager: &CronManager,
    parts: &[&str],
    page: usize,
) -> Result<Option<String>, String> {
    if parts.len() >= 4 {
        let slot: usize = parts[2].parse().unwrap_or(0);
        let fp = parts[3];
        let jobs = manager.list_jobs().await?;
        let start = page * PAGE_SIZE;
        if let Some(job) = jobs.get(start + slot) {
            if fingerprint(&job.id) == fp {
                let new_state = !job.enabled;
                let _ = manager.set_enabled(&job.id, new_state).await?;
                let state_str = if new_state {
                    t!("bot.cron_state_enabled")
                } else {
                    t!("bot.cron_state_disabled")
                };
                return Ok(Some(t!(
                    "bot.cron_toggle_success",
                    title = job.title,
                    state = state_str
                )));
            } else {
                return Ok(Some(t!("bot.cron_toggle_mismatch")));
            }
        }
    }
    Ok(None)
}

pub(crate) async fn handle_cron_callback(
    bot: &Bot,
    chat_id: ChatId,
    message_id: MessageId,
    data: &str,
    manager: &CronManager,
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
            update_message(bot, chat_id, message_id, manager, next_page, None).await?;
        }
        "n" => {
            update_message(bot, chat_id, message_id, manager, page + 1, None).await?;
        }
        "r" => {
            let note = t!("bot.cron_refreshed_note");
            update_message(bot, chat_id, message_id, manager, page, Some(&note)).await?;
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
            update_message(bot, chat_id, message_id, manager, page, Some(&note)).await?;
        }
        "t" => {
            let toggle_note = handle_toggle_action(manager, &parts, page).await?;
            update_message(bot, chat_id, message_id, manager, page, toggle_note.as_deref()).await?;
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
) -> Result<(), String> {
    let (text, markup) = build_cron_page(manager, page, note).await?;
    let _ = bot.edit_message_text(chat_id, message_id, text)
        .reply_markup(markup)
        .await;
    Ok(())
}
