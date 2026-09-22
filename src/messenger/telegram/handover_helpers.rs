//! # Telegram Topic Handover Distillation Helpers
//!
//! ## Overview
//! Provides helper functions for extracting, parsing, and distilling topic conversation history
//! from previous session logs. Distills objectives, recent decisions, and progress snippets.
//!
//! ## Search Tags
//! #handover-helpers, #context-distillation, #history-parser, #forum-topics

use std::path::Path;
use std::fs::File;
use std::io::{BufRead, BufReader};
use crate::messenger::telegram::history::TelegramHistoryEntry;

pub fn extract_topic_history(working_dir: &Path, session_id: &str) -> Vec<TelegramHistoryEntry> {
    if session_id.trim().is_empty() { return Vec::new(); }
    let path = working_dir.join("brain").join(session_id).join("telegram_history.jsonl");
    let file = match File::open(&path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let mut reader = BufReader::new(file);
    let mut entries = Vec::new();
    let mut raw_line = Vec::new();

    while let Ok(n) = reader.read_until(b'\n', &mut raw_line) {
        if n == 0 { break; }
        let line = String::from_utf8_lossy(&raw_line);
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            if let Ok(entry) = serde_json::from_str::<TelegramHistoryEntry>(trimmed) {
                let text = entry.text.trim();
                if entry.is_success && entry.error.is_none() && !text.is_empty() && !is_meta_cmd(text) {
                    entries.push(entry);
                }
            }
        }
        raw_line.clear();
    }
    entries
}

pub fn is_meta_cmd(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    if lower.starts_with("[topic context handover]") || lower.contains("instructions for continuing this session") {
        return true;
    }
    let raw_cmd = lower.split_whitespace().next().unwrap_or("");
    let cmd = raw_cmd.split('@').next().unwrap_or(raw_cmd);
    matches!(cmd, "/new" | "/reset" | "/stop" | "/abort" | "/stop_all" | "/model" | "/status" | "/usage" | "/memory" | "/help" | "/diagnose" | "/restart" | "/upgrade")
}

pub fn is_greeting(line: &str) -> bool {
    let l = line.trim().to_lowercase();
    matches!(l.as_str(), "hi" | "hello" | "hey" | "안녕" | "안녕하세요" | "시작" | "start" | "hi!" | "hello!" | "안녕하세요!" | "hi tuner" | "hello tuner")
}

pub fn extract_topic_objective(entries: &[TelegramHistoryEntry], topic_name: Option<&str>, prior_obj: Option<&str>) -> String {
    if let Some(po) = prior_obj {
        let clean_po = po.trim();
        if !clean_po.is_empty() && clean_po != "Ongoing topic tasks and technical discussions" {
            return clean_po.to_string();
        }
    }

    let user_msg = entries.iter()
        .find(|e| e.sender.eq_ignore_ascii_case("user"))
        .map(|e| {
            let non_greetings: Vec<&str> = e.text.lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty() && !is_greeting(l))
                .collect();
            let combined = if !non_greetings.is_empty() {
                non_greetings.join(" ")
            } else {
                e.text.trim().to_string()
            };
            if combined.len() > 180 { format!("{}...", &combined[..180]) } else { combined }
        });

    match (topic_name, user_msg) {
        (Some(n), Some(u)) if !n.is_empty() => format!("[Topic: {}] {}", n, u),
        (Some(n), None) if !n.is_empty() => format!("Topic focus: {}", n),
        (_, Some(u)) => u,
        _ => "Ongoing topic tasks and technical discussions".to_string(),
    }
}

pub fn is_boilerplate_or_header(line: &str) -> bool {
    let l = line.trim();
    l.starts_with('#')
        || l.starts_with("```")
        || l.starts_with("---")
        || l.starts_with("[TOPIC")
        || l.contains("Seamlessly continue")
        || l.contains("very first response")
        || l.contains("Acknowledge that this topic")
        || l.contains("INSTRUCTIONS FOR")
}

pub fn is_decision_line(line: &str) -> bool {
    if is_boilerplate_or_header(line) { return false; }
    let l = line.to_lowercase();
    let kw = [
        "결정", "확정", "합의", "선택", "채택", "적용", "완료", "하기로", "방식으로",
        "decided", "agreed", "chosen", "will use", "adopted", "plan is", "confirmed", "implemented"
    ];
    kw.iter().any(|k| l.contains(k))
}

fn merge_prior_decisions(decisions: Vec<String>, prior_decisions: &[String]) -> Vec<String> {
    if prior_decisions.is_empty() { return decisions; }
    let mut merged = Vec::new();
    for pd in prior_decisions {
        let clean = pd.trim();
        if !clean.is_empty()
            && !clean.starts_with("No explicit technical decisions")
            && !merged.contains(&clean.to_string())
            && !decisions.contains(&clean.to_string())
        {
            merged.push(clean.to_string());
        }
    }
    merged.extend(decisions);
    if merged.len() > 5 {
        let start = merged.len() - 5;
        merged[start..].to_vec()
    } else {
        merged
    }
}

fn extract_fallback_steps(entries: &[TelegramHistoryEntry]) -> Vec<String> {
    let mut steps = Vec::new();
    for entry in entries.iter().rev().filter(|e| !e.sender.eq_ignore_ascii_case("user")) {
        for line in entry.text.lines().rev() {
            let clean = line.trim().trim_start_matches(|c| c == '-' || c == '*' || c == '•').trim();
            if !is_boilerplate_or_header(clean)
                && (clean.starts_with("1.") || clean.starts_with("2.") || clean.contains("Step") || clean.contains("단계"))
                && clean.len() > 10
            {
                let item = if clean.len() > 150 { format!("{}...", &clean[..150]) } else { clean.to_string() };
                if !steps.contains(&item) { steps.push(item); }
            }
            if steps.len() >= 3 { break; }
        }
        if steps.len() >= 3 { break; }
    }
    steps.reverse();
    steps
}

pub fn extract_agreed_decisions(entries: &[TelegramHistoryEntry], prior_decisions: &[String]) -> Vec<String> {
    let mut decisions = Vec::new();
    for entry in entries.iter().rev() {
        for line in entry.text.lines().rev() {
            let clean = line.trim().trim_start_matches(|c| c == '-' || c == '*' || c == '•').trim();
            if clean.len() >= 6 && is_decision_line(clean) {
                let item = if clean.len() > 150 { format!("{}...", &clean[..150]) } else { clean.to_string() };
                if !decisions.contains(&item) { decisions.push(item); }
            }
            if decisions.len() >= 5 { break; }
        }
        if decisions.len() >= 5 { break; }
    }
    decisions.reverse();
    decisions = merge_prior_decisions(decisions, prior_decisions);

    if decisions.is_empty() {
        decisions = extract_fallback_steps(entries);
    }
    if decisions.is_empty() {
        decisions.push("No explicit technical decisions recorded in previous session; continuing with baseline.".to_string());
    }
    decisions
}

pub fn extract_current_progress(entries: &[TelegramHistoryEntry], prior_prog: Option<&str>) -> String {
    let last_bot = entries.iter().rev().find(|e| !e.sender.eq_ignore_ascii_case("user") && !is_boilerplate_or_header(&e.text));
    let last_user = entries.iter().rev().find(|e| e.sender.eq_ignore_ascii_case("user") && !is_meta_cmd(&e.text));
    let mut parts = Vec::new();

    if let Some(bot) = last_bot {
        let lines: Vec<&str> = bot.text.lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !is_boilerplate_or_header(l))
            .take(3)
            .collect();
        let text = lines.join(" ");
        let snip = if text.len() > 200 { format!("{}...", &text[..200]) } else { text };
        if !snip.is_empty() { parts.push(format!("Last assistant status: {}", snip)); }
    }
    if let Some(usr) = last_user {
        let lines: Vec<&str> = usr.text.lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !is_greeting(l))
            .take(3)
            .collect();
        let text = lines.join(" ");
        let snip = if text.len() > 200 { format!("{}...", &text[..200]) } else { text };
        if !snip.is_empty() { parts.push(format!("Latest user request: {}", snip)); }
    }

    if parts.is_empty() {
        if let Some(pp) = prior_prog {
            let clean_pp = pp.trim();
            if !clean_pp.is_empty() { return clean_pp.to_string(); }
        }
        "Initial session discussion in progress.".to_string()
    } else {
        parts.join("\n")
    }
}
