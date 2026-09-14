//! # Model Quota and Usage Telegram Command
//!
//! Handles the `/usage` slash command by querying the Antigravity CLI quota modal,
//! parsing model quota percentages and limits, and formatting the output for Telegram.

use std::time::Duration;
use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::session::manager::SessionManager;
use crate::cli::antigravity::AntigravityCli;
use crate::messenger::telegram::topic_cache::TopicNameCache;
use crate::cli::antigravity::pty_spawner::strip_ansi;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuotaLimit {
    pub pct: f64,
    pub detail: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelQuotaGroup {
    pub weekly: Option<QuotaLimit>,
    pub five_hour: Option<QuotaLimit>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuotaReport {
    pub account: Option<String>,
    pub gemini: Option<ModelQuotaGroup>,
    pub claude: Option<ModelQuotaGroup>,
    pub raw_fallback: String,
}

pub fn format_progress_bar(pct: f64) -> String {
    let total = 20;
    let filled = ((pct / 100.0) * (total as f64)).round().clamp(0.0, total as f64) as usize;
    format!("[{}{}]", "█".repeat(filled), "░".repeat(total - filled))
}

pub fn parse_limit(text: &str, header: &str) -> Option<QuotaLimit> {
    let pos = text.find(header)?;
    let slice = &text[pos..];
    let end_pos = slice.find("\n\n").unwrap_or_else(|| slice.len().min(400));
    let chunk = &slice[..end_pos];

    let pct_pos = chunk.find('%')?;
    let pct = chunk[..pct_pos]
        .split_whitespace()
        .last()?
        .trim_matches(|c: char| !c.is_ascii_digit() && c != '.')
        .parse::<f64>()
        .ok()?;

    let mut lines = chunk.lines().map(|l| l.trim());
    lines.next();
    let mut detail = String::new();
    for line in lines {
        if line.is_empty() || line.starts_with('[') || line.starts_with('|') { continue; }
        if line.contains("Limit") || line.contains("MODELS") || line.contains("Models") { break; }
        detail = line.to_string();
        break;
    }

    Some(QuotaLimit { pct, detail })
}

pub fn parse_group_block(text: &str, group_marker: &str) -> Option<ModelQuotaGroup> {
    let pos = text.find(group_marker)?;
    let slice = &text[pos..];
    let mut lines = slice.lines();
    lines.next();

    let mut block = String::new();
    for line in lines {
        if line.contains("MODELS") { break; }
        block.push_str(line);
        block.push('\n');
    }

    let weekly = parse_limit(&block, "Weekly Limit");
    let five_hour = parse_limit(&block, "Five Hour Limit")
        .or_else(|| parse_limit(&block, "5-Hour Limit"))
        .or_else(|| parse_limit(&block, "5 Hour Limit"));

    if weekly.is_some() || five_hour.is_some() {
        Some(ModelQuotaGroup { weekly, five_hour })
    } else {
        None
    }
}

pub fn parse_quota_report(raw: &str) -> QuotaReport {
    let stripped = strip_ansi(raw);
    let account = stripped.lines()
        .find(|l| l.contains("Account:") || l.contains("Account :"))
        .and_then(|l| l.split("Account").nth(1))
        .map(|s| s.trim_start_matches(':').trim().to_string())
        .filter(|s| !s.is_empty());

    let gemini = parse_group_block(&stripped, "GEMINI");
    let claude = parse_group_block(&stripped, "CLAUDE");

    QuotaReport { account, gemini, claude, raw_fallback: stripped }
}

pub(crate) async fn query_quota_from_pty(
    cli: &AntigravityCli,
    session_id: &str,
) -> Result<QuotaReport, String> {
    cli.ensure_interactive_ready(session_id).await?;
    let start_len = cli.sessions.get_output_len(session_id).await.unwrap_or(0);
    cli.sessions.write_to_session(session_id, "/usage\r").await?;

    let mut accumulated = Vec::new();
    let start = std::time::Instant::now();
    let mut sent_pagedown = false;

    while start.elapsed() < Duration::from_millis(3500) {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Some(bytes) = cli.sessions.get_output_from(session_id, start_len).await {
            accumulated = bytes;
            let text = String::from_utf8_lossy(&accumulated);
            if text.contains("CLAUDE AND GPT MODELS") {
                let after_claude = text.split("CLAUDE AND GPT MODELS").nth(1).unwrap_or("");
                if after_claude.contains("Five Hour Limit") && after_claude.contains('%') {
                    break;
                }
                if !sent_pagedown {
                    let _ = cli.sessions.write_to_session(session_id, "\x1b[6~").await;
                    sent_pagedown = true;
                }
            } else if (text.contains("GEMINI MODELS") || text.contains("Models & Quota")) && !sent_pagedown {
                let _ = cli.sessions.write_to_session(session_id, "\x1b[6~").await;
                sent_pagedown = true;
            }
        }
    }

    let _ = cli.sessions.write_to_session(session_id, "\x1b").await;
    let text = String::from_utf8_lossy(&accumulated);
    if !text.contains("Models & Quota") && !text.contains("GEMINI MODELS") {
        return Err("Quota modal did not appear or timed out".to_string());
    }

    Ok(parse_quota_report(&text))
}

fn render_group_markdown(icon: &str, title: &str, group: &ModelQuotaGroup) -> String {
    let mut out = format!("\n{} <b>{}</b>\n", icon, title);
    match (&group.weekly, &group.five_hour) {
        (Some(w), Some(f)) => {
            out.push_str(&format!("├ 🔹 Weekly Limit: <code>{}</code> {:.1}%\n", format_progress_bar(w.pct), w.pct));
            if !w.detail.is_empty() { out.push_str(&format!("│   └ {}\n", html_escape::encode_safe(&w.detail))); }
            out.push_str(&format!("└ 🔹 5-Hour Limit: <code>{}</code> {:.1}%\n", format_progress_bar(f.pct), f.pct));
            if !f.detail.is_empty() { out.push_str(&format!("    └ {}\n", html_escape::encode_safe(&f.detail))); }
        }
        (Some(w), None) => {
            out.push_str(&format!("└ 🔹 Weekly Limit: <code>{}</code> {:.1}%\n", format_progress_bar(w.pct), w.pct));
            if !w.detail.is_empty() { out.push_str(&format!("    └ {}\n", html_escape::encode_safe(&w.detail))); }
        }
        (None, Some(f)) => {
            out.push_str(&format!("└ 🔹 5-Hour Limit: <code>{}</code> {:.1}%\n", format_progress_bar(f.pct), f.pct));
            if !f.detail.is_empty() { out.push_str(&format!("    └ {}\n", html_escape::encode_safe(&f.detail))); }
        }
        (None, None) => {}
    }
    out
}

pub(crate) fn render_quota_message(report: &QuotaReport) -> String {
    let mut text = String::from("📊 <b>[tuner] Model Quota & Usage</b>\n");
    if let Some(ref acc) = report.account {
        text.push_str(&format!("👤 <code>{}</code>\n", html_escape::encode_safe(acc)));
    }

    let mut rendered = false;
    if let Some(ref gemini) = report.gemini {
        text.push_str(&render_group_markdown("🤖", "GEMINI MODELS (Flash, Pro)", gemini));
        rendered = true;
    }
    if let Some(ref claude) = report.claude {
        text.push_str(&render_group_markdown("🧠", "CLAUDE AND GPT MODELS (Opus, Sonnet, GPT)", claude));
        rendered = true;
    }

    if !rendered {
        text.push_str(&format!("\n<pre>{}</pre>", html_escape::encode_safe(report.raw_fallback.trim())));
    }
    text
}

async fn resolve_session_id(
    bot: &Bot,
    msg: &Message,
    config: &CliConfig,
    sessions: &SessionManager,
    cli: &AntigravityCli,
) -> Option<String> {
    let default_model = config.model.as_deref().unwrap_or("antigravity-default");
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));

    let (mut sess, _) = match sessions.resolve_session(&key, &config.provider, default_model).await {
        Ok(s) => s,
        Err(e) => {
            let _ = super::commands::send_reply(bot, msg, format!("❌ Error resolving session: {}", e)).await;
            return None;
        }
    };

    match crate::telegram::session_init::initialize_session_if_needed(bot, msg, sessions, &mut sess, cli, config).await {
        Ok(s) if !s.is_empty() => Some(s),
        Ok(_) => {
            let _ = super::commands::send_reply(bot, msg, "❌ Could not initialize session for quota check").await;
            None
        }
        Err(e) => {
            let _ = super::commands::send_reply(bot, msg, format!("❌ Error initializing session: {}", e)).await;
            None
        }
    }
}

async fn send_reply_html(bot: &Bot, msg: &Message, html: &str) {
    let mut req = bot.send_message(msg.chat.id, html).parse_mode(teloxide::types::ParseMode::Html);
    if let Some(tid) = msg.thread_id { req = req.message_thread_id(tid); }
    if req.await.is_err() {
        let _ = super::commands::send_reply(bot, msg, html).await;
    }
}

pub(crate) async fn handle_usage_command(
    bot: &Bot,
    msg: &Message,
    config: &CliConfig,
    sessions: &SessionManager,
    cli: &AntigravityCli,
    _topic_cache: &TopicNameCache,
) -> Result<(), teloxide::RequestError> {
    let Some(sid) = resolve_session_id(bot, msg, config, sessions, cli).await else {
        return Ok(());
    };

    let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
    let _g = super::typing::TelegramTypingGuard::new(bot.clone(), tok, msg).await;

    let report = match query_quota_from_pty(cli, &sid).await {
        Ok(r) => r,
        Err(e) => {
            drop(_g);
            let _ = super::commands::send_reply(bot, msg, format!("⚠️ Could not retrieve quota from session: {}", e)).await;
            return Ok(());
        }
    };
    drop(_g);

    send_reply_html(bot, msg, &render_quota_message(&report)).await;
    Ok(())
}
