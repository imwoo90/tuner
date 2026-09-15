//! # Remote Development Services Telegram Command Controller
//!
//! Renders the `/remote` dashboard message, manages inline action buttons,
//! and dispatches user actions (start, stop, restart, refresh).
//!
//! ## Search Tags
//! #remote-command, #telegram-dashboard, #service-toggle

use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
pub use super::commands_remote_hub::*;

fn build_antigravity_lines(status: &RemoteHubStatus, lines: &mut Vec<String>) {
    match status.antigravity {
        ServiceStatus::Active => {
            lines.push("• <b>Google Antigravity</b> : 🟢 <b>Active</b>".to_string());
            if let Some(ref url) = status.antigravity_url {
                lines.push(format!("  └ 🔗 <b>URL</b>: <code>{}</code>", url));
            }
            if let (Some(uptime), Some(pid)) = (&status.antigravity_uptime, status.antigravity_pid) {
                lines.push(format!("  └ ⏱️ <b>Uptime</b>: {} (PID: {})", uptime, pid));
            }
        }
        ServiceStatus::Inactive => {
            lines.push("• <b>Google Antigravity</b> : 🔴 <b>Inactive</b>".to_string());
        }
        ServiceStatus::NotInstalled => {
            lines.push("• <b>Google Antigravity</b> : ⚠️ <b>Not Installed</b>".to_string());
        }
    }
}

fn build_network_lines(status: &RemoteHubStatus, lines: &mut Vec<String>) {
    match status.tailscale {
        ServiceStatus::Active => {
            if let Some(ref ip) = status.tailscale_ip {
                lines.push(format!("• <b>Tailscale</b>          : 🟢 <b>Connected</b> (<code>{}</code>)", ip));
            } else {
                lines.push("• <b>Tailscale</b>          : 🟢 <b>Connected</b>".to_string());
            }
        }
        ServiceStatus::Inactive => {
            lines.push("• <b>Tailscale</b>          : 🔴 <b>Inactive</b> (Disconnected)".to_string());
        }
        ServiceStatus::NotInstalled => {
            lines.push("• <b>Tailscale</b>          : ⚠️ <b>Not Installed</b>".to_string());
        }
    }

    match status.ssh {
        ServiceStatus::Active => {
            lines.push("• <b>SSH Service</b>        : 🟢 <b>Listening</b> (<code>Port 22</code>)".to_string());
        }
        ServiceStatus::Inactive => {
            lines.push("• <b>SSH Service</b>        : 🔴 <b>Inactive</b>".to_string());
        }
        ServiceStatus::NotInstalled => {
            lines.push("• <b>SSH Service</b>        : ⚠️ <b>Not Installed</b>".to_string());
        }
    }
}

fn build_dashboard_keyboard(status: &RemoteHubStatus) -> InlineKeyboardMarkup {
    let mut keyboard = Vec::new();

    let mut row1 = Vec::new();
    match status.antigravity {
        ServiceStatus::Active => {
            if let Some(ref url) = status.antigravity_url {
                if let Ok(parsed_url) = reqwest::Url::parse(url) {
                    row1.push(InlineKeyboardButton::url("🔗 Web IDE 열기", parsed_url));
                }
            }
            row1.push(InlineKeyboardButton::callback("⏹️ Antigravity 중지", "rem:agy:stop"));
        }
        ServiceStatus::Inactive => {
            row1.push(InlineKeyboardButton::callback("🚀 Antigravity 시작", "rem:agy:start"));
        }
        ServiceStatus::NotInstalled => {
            row1.push(InlineKeyboardButton::callback("📥 Antigravity 설치 안내", "rem:agy:install"));
        }
    }
    if !row1.is_empty() {
        keyboard.push(row1);
    }

    let mut row2 = Vec::new();
    if status.antigravity == ServiceStatus::Active {
        row2.push(InlineKeyboardButton::callback("🔄 Antigravity 재시작", "rem:agy:restart"));
    }
    if status.tailscale == ServiceStatus::Inactive {
        row2.push(InlineKeyboardButton::callback("🔒 Tailscale 연결", "rem:ts:start"));
    }
    if !row2.is_empty() {
        keyboard.push(row2);
    }

    keyboard.push(vec![InlineKeyboardButton::callback("🔄 새로고침", "rem:refresh")]);
    InlineKeyboardMarkup::new(keyboard)
}

/// Renders HTML formatted dashboard text and inline keyboard markup.
pub fn render_remote_dashboard(status: &RemoteHubStatus) -> (String, InlineKeyboardMarkup) {
    let mut lines = vec!["🌐 <b>Remote Services</b>\n".to_string()];
    build_antigravity_lines(status, &mut lines);
    build_network_lines(status, &mut lines);
    (lines.join("\n"), build_dashboard_keyboard(status))
}

/// Handles the `/remote` slash command by rendering and sending the initial dashboard.
pub async fn handle_remote_command(
    bot: &Bot,
    msg: &Message,
) -> Result<(), teloxide::RequestError> {
    let status = query_remote_hub_status();
    let (text, markup) = render_remote_dashboard(&status);

    let mut req = bot.send_message(msg.chat.id, text).parse_mode(ParseMode::Html);
    if let Some(tid) = msg.thread_id {
        req = req.message_thread_id(tid);
    }
    req.reply_markup(markup).await?;
    Ok(())
}

/// Handles callback query interactions (`rem:*`) for remote service toggling.
pub async fn handle_remote_callback(
    bot: &Bot,
    msg: &Message,
    data: &str,
) {
    match data {
        "rem:agy:start" => {
            let _ = start_antigravity_remote().await;
        }
        "rem:agy:stop" => {
            let _ = stop_antigravity_remote().await;
        }
        "rem:agy:restart" => {
            let _ = stop_antigravity_remote().await;
            let _ = start_antigravity_remote().await;
        }
        "rem:ts:start" => {
            let _ = std::process::Command::new("tailscale").arg("up").output();
        }
        _ => {}
    }

    let status = query_remote_hub_status();
    let (text, markup) = render_remote_dashboard(&status);

    let _ = bot
        .edit_message_text(msg.chat.id, msg.id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(markup)
        .await;
}
