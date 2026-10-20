//! # Model Quota and Usage Telegram Command
//!
//! ## Overview
//! Handles the `/usage` slash command by querying the Antigravity CLI quota modal,
//! parsing model quota percentages and limits, and formatting the output for Telegram.
//!
//! ## Search Tags
//! #usage-command, #quota-modal, #token-usage

use std::time::{Duration, Instant};
use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::session::{key::SessionKey, manager::SessionManager};
use crate::cli::antigravity::{pty_spawner::strip_ansi, AntigravityCli};
use crate::messenger::telegram::topic_cache::TopicNameCache;
use super::commands::send_reply;

#[derive(Clone, Debug, Default, PartialEq)]
/// Represents `QuotaLimit` (quota limit).
pub struct QuotaLimit {
    /// Pct.
    pub pct: f64,
    /// Detail.
    pub detail: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
/// Represents `ModelQuotaGroup` (model quota group).
pub struct ModelQuotaGroup {
    /// Weekly.
    pub weekly: Option<QuotaLimit>,
    /// Five hour.
    pub five_hour: Option<QuotaLimit>,
}

#[derive(Clone, Debug, Default, PartialEq)]
/// Represents `QuotaReport` (quota report).
pub struct QuotaReport {
    /// Account.
    pub account: Option<String>,
    /// Gemini.
    pub gemini: Option<ModelQuotaGroup>,
    /// Claude.
    pub claude: Option<ModelQuotaGroup>,
    /// Raw fallback.
    pub raw_fallback: String,
}

/// Executes `format_progress_bar` (format progress bar).
pub fn format_progress_bar(pct: f64) -> String {
    let total = 10;
    let filled = ((pct / 100.0) * (total as f64)).round().clamp(0.0, total as f64) as usize;
    format!("[{}{}]", "█".repeat(filled), "░".repeat(total - filled))
}

/// Executes `parse_limit` (parse limit).
pub fn parse_limit(text: &str, header: &str) -> Option<QuotaLimit> {
    let pos = text.find(header)?;
    let slice = &text[pos..];
    let end_pos = slice.find("\n\n").unwrap_or_else(|| slice.len().min(400));
    let chunk = &slice[..end_pos];

    if let Some(pct_pos) = chunk.find('%') {
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

        return Some(QuotaLimit { pct, detail });
    }

    for line in chunk.lines().skip(1).map(|l| l.trim()) {
        if line.starts_with("Disabled") {
            return Some(QuotaLimit {
                pct: 0.0,
                detail: line.to_string(),
            });
        }
    }

    None
}

/// Executes `parse_group_block` (parse group block).
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

/// Executes `parse_quota_report` (parse quota report).
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

/// Evaluates whether the Antigravity quota modal has finished loading its model groups.
pub fn is_quota_modal_ready(text: &str) -> bool {
    let clean = strip_ansi(text);
    if clean.contains("GEMINI MODELS") && clean.contains("CLAUDE AND GPT MODELS") {
        let after = clean.split("CLAUDE AND GPT MODELS").nth(1).unwrap_or("");
        let has_5h = ["Five Hour Limit", "5-Hour Limit", "5 Hour Limit"]
            .iter()
            .any(|p| after.contains(p));
        return has_5h
            && (after.contains("Refreshes")
                || after.contains("Resets")
                || after.contains("Disabled"));
    }
    false
}

async fn read_quota_modal_pass(
    cli: &AntigravityCli,
    session_id: &str,
    settle_ms: u64,
) -> Result<String, String> {
    let start_len = cli.sessions.get_output_len(session_id).await.unwrap_or(0);
    cli.sessions.write_to_session(session_id, "/usage\r").await?;

    let mut acc = Vec::new();
    let start = Instant::now();
    let mut ready_at: Option<Instant> = None;

    while start.elapsed() < Duration::from_millis(4000) {
        tokio::time::sleep(Duration::from_millis(80)).await;
        if let Some(bytes) = cli.sessions.get_output_from(session_id, start_len).await {
            acc = bytes;
            let raw = String::from_utf8_lossy(&acc);
            if is_quota_modal_ready(&raw) {
                let first = *ready_at.get_or_insert_with(Instant::now);
                if raw.contains("Loading quota") || settle_ms == 0 || first.elapsed() >= Duration::from_millis(settle_ms) {
                    break;
                }
            }
        }
    }

    let _ = cli.sessions.write_to_session(session_id, "\x1b").await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    Ok(String::from_utf8_lossy(&acc).to_string())
}

pub(crate) async fn query_quota_from_pty(
    cli: &AntigravityCli,
    session_id: &str,
) -> Result<QuotaReport, String> {
    cli.ensure_interactive_ready(session_id).await?;
    let first = read_quota_modal_pass(cli, session_id, 550).await?;

    // Warm-cache `/usage` draws stale cached values first and updates digits in-place via
    // cursor diff ~300ms later; re-opening after 550ms forces a clean redraw of fresh quota.
    let text = if !first.contains("Loading quota") && is_quota_modal_ready(&first) {
        match read_quota_modal_pass(cli, session_id, 0).await {
            Ok(second) if is_quota_modal_ready(&second) => second,
            _ => first,
        }
    } else {
        first
    };

    if !text.contains("Models & Quota") && !text.contains("GEMINI MODELS") {
        return Err("Quota modal did not appear or timed out".to_string());
    }
    Ok(parse_quota_report(&text))
}

fn push_limit_line(out: &mut String, branch: &str, sub: &str, label: &str, lim: &QuotaLimit) {
    out.push_str(&format!("{} 🔹 {}: <code>{}</code> {:.1}%\n", branch, label, format_progress_bar(lim.pct), lim.pct));
    if !lim.detail.is_empty() {
        out.push_str(&format!("{}   └ {}\n", sub, html_escape::encode_safe(&lim.detail)));
    }
}

fn render_group_markdown(icon: &str, title: &str, group: &ModelQuotaGroup) -> String {
    let mut out = format!("\n{} <b>{}</b>\n", icon, title);
    match (&group.weekly, &group.five_hour) {
        (Some(w), Some(f)) => {
            push_limit_line(&mut out, "├", "│", "Weekly Limit", w);
            push_limit_line(&mut out, "└", " ", "5-Hour Limit", f);
        }
        (Some(w), None) => push_limit_line(&mut out, "└", " ", "Weekly Limit", w),
        (None, Some(f)) => push_limit_line(&mut out, "└", " ", "5-Hour Limit", f),
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
    let model = config.model.as_deref().unwrap_or("antigravity-default");
    let key = SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));

    let (mut sess, _) = match sessions.resolve_session(&key, &config.provider, model).await {
        Ok(s) => s,
        Err(e) => {
            let _ = send_reply(bot, msg, format!("❌ Error resolving session: {}", e)).await;
            return None;
        }
    };

    match crate::telegram::session_init::initialize_session_if_needed(bot, msg, sessions, &mut sess, cli, config).await {
        Ok(s) if !s.is_empty() => Some(s),
        Ok(_) => {
            let _ = send_reply(bot, msg, "❌ Could not initialize session for quota check").await;
            None
        }
        Err(e) => {
            let _ = send_reply(bot, msg, format!("❌ Error initializing session: {}", e)).await;
            None
        }
    }
}

async fn send_reply_html(bot: &Bot, msg: &Message, html: &str) {
    let mut req = bot.send_message(msg.chat.id, html).parse_mode(teloxide::types::ParseMode::Html);
    if let Some(tid) = msg.thread_id { req = req.message_thread_id(tid); }
    if req.await.is_err() {
        let _ = send_reply(bot, msg, html).await;
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
            let _ = send_reply(bot, msg, format!("⚠️ Could not retrieve quota from session: {}", e)).await;
            return Ok(());
        }
    };
    drop(_g);

    send_reply_html(bot, msg, &render_quota_message(&report)).await;
    Ok(())
}
