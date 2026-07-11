use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId};

use crate::cron::manager::{CronJob, CronManager};

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
    let status = if job.enabled { "🟢 활성" } else { "🔴 비활성" };
    let last_run = job.last_run_status.as_deref().unwrap_or("없음");
    let line = format!(
        "{}. {} ({})\n   ↳ 스케줄: `{}` | 마지막: `{}`\n   ↳ 작업 폴더: `{}`",
        number, job.title, status, job.schedule, last_run, job.task_folder
    );
    let button_text = if job.enabled {
        format!("{}번 비활성화", number)
    } else {
        format!("{}번 활성화", number)
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
        nav_row.push(InlineKeyboardButton::callback("◀ 이전", format!("crn:p:{}", current_page)));
    }
    nav_row.push(InlineKeyboardButton::callback("🔄 새로고침", format!("crn:r:{}", current_page)));
    if current_page < total_pages - 1 {
        nav_row.push(InlineKeyboardButton::callback("다음 ▶", format!("crn:n:{}", current_page)));
    }
    keyboard.push(nav_row);

    keyboard.push(vec![
        InlineKeyboardButton::callback("🟢 전체 켜기", format!("crn:ao:{}", current_page)),
        InlineKeyboardButton::callback("🔴 전체 끄기", format!("crn:af:{}", current_page)),
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
        let text = "🤖 [우덕터] 등록된 크론 작업이 없습니다.\n\n\
                    크론 작업을 생성하여 ~/.ductor/cron_jobs.json 에 등록할 수 있습니다."
            .to_string();
        return Ok((text, InlineKeyboardMarkup::new(Vec::<Vec<InlineKeyboardButton>>::new())));
    }

    let total_pages = (jobs.len() + PAGE_SIZE - 1) / PAGE_SIZE;
    let current_page = page.min(total_pages - 1);
    let start = current_page * PAGE_SIZE;
    let end = (start + PAGE_SIZE).min(jobs.len());
    let page_jobs = &jobs[start..end];

    let mut lines = vec!["🤖 [우덕터] 크론 작업 목록:".to_string(), "".to_string()];
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
    lines.push(format!("페이지: {} / {}", current_page + 1, total_pages));

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
                let state_str = if new_state { "활성화" } else { "비활성화" };
                return Ok(Some(format!("👉 '{}' 작업이 {}되었습니다.", job.title, state_str)));
            } else {
                return Ok(Some("❌ 작업의 정보가 일치하지 않습니다.".to_string()));
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
            update_message(bot, chat_id, message_id, manager, page, Some("🔄 최신 상태로 새로고침되었습니다.")).await?;
        }
        "ao" | "af" => {
            let enabled = action == "ao";
            let changed_count = manager.set_all_enabled(enabled).await?;
            let note = Some(if enabled {
                format!("🟢 {}개 작업이 모두 활성화되었습니다.", changed_count)
            } else {
                format!("🔴 {}개 작업이 모두 비활성화되었습니다.", changed_count)
            });
            update_message(bot, chat_id, message_id, manager, page, note.as_deref()).await?;
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
