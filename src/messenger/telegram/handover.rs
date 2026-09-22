//! # Telegram Topic Session Handover Manager
//!
//! ## Overview
//! Extracts, distills, and formats topic-specific context across `/new` and `/reset` commands.
//! Preserves topic objective, agreed decisions, and current progress without context leakage.
//!
//! ## Collaboration Graph
//! - Invoked by [`super::commands_session::handle_new_command`] when resetting a topic session.
//! - Injected into [`super::session_init::initialize_session_with_prompt`] as the initial boot prompt.
//! - Delegates history distillation algorithms to [`super::handover_helpers`].
//!
//! ## Search Tags
//! #topic-handover, #session-reset, #context-compression, #forum-topics

use std::path::Path;
use crate::messenger::telegram::history::{TelegramHistoryEntry, TopicMetadata};
pub use super::handover_helpers::*;

/// Structured representation of compressed topic context for handover.
#[derive(Clone, Debug, PartialEq)]
pub struct TopicHandoverContext {
    pub topic_id: Option<i64>,
    pub topic_name: Option<String>,
    pub previous_session_id: String,
    pub topic_objective: String,
    pub agreed_decisions: Vec<String>,
    pub current_progress: String,
    pub handover_prompt: String,
}

pub fn parse_prior_summary(summary: &str) -> (Option<String>, Vec<String>, Option<String>) {
    let mut obj = None;
    let mut decs = Vec::new();
    let mut prog = None;
    let mut current_section = 0;
    let mut obj_lines = Vec::new();
    let mut prog_lines = Vec::new();

    for line in summary.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Objective:") {
            current_section = 1;
            let o = rest.trim();
            if !o.is_empty() { obj_lines.push(o.to_string()); }
        } else if let Some(rest) = trimmed.strip_prefix("Decisions:") {
            current_section = 2;
            let d = rest.trim();
            if !d.is_empty() {
                for item in d.split(';') {
                    let c = item.trim();
                    if !c.is_empty() && !c.starts_with("No explicit technical decisions") {
                        decs.push(c.to_string());
                    }
                }
            }
        } else if let Some(rest) = trimmed.strip_prefix("Progress:") {
            current_section = 3;
            let p = rest.trim();
            if !p.is_empty() { prog_lines.push(p.to_string()); }
        } else if current_section == 1 && !trimmed.is_empty() {
            obj_lines.push(trimmed.to_string());
        } else if current_section == 2 && !trimmed.is_empty() {
            let c = trimmed.trim_start_matches(|ch| ch == '-' || ch == '*' || ch == '•').trim();
            if !c.is_empty() && !c.starts_with("No explicit technical decisions") {
                decs.push(c.to_string());
            }
        } else if current_section == 3 && !trimmed.is_empty() {
            prog_lines.push(trimmed.to_string());
        }
    }

    if !obj_lines.is_empty() { obj = Some(obj_lines.join(" ")); }
    if !prog_lines.is_empty() { prog = Some(prog_lines.join("\n")); }
    (obj, decs, prog)
}

pub fn format_recent_excerpts(entries: &[TelegramHistoryEntry], max_count: usize) -> Vec<String> {
    let clean_entries: Vec<&TelegramHistoryEntry> = entries.iter()
        .filter(|e| !is_meta_cmd(&e.text) && !is_boilerplate_or_header(&e.text))
        .collect();
    let start = clean_entries.len().saturating_sub(max_count);
    clean_entries[start..].iter().map(|e| {
        let role = if e.sender.eq_ignore_ascii_case("user") { "User" } else { "Assistant" };
        let t = e.text.trim();
        let clean = if t.len() > 250 { format!("{}... [truncated]", &t[..250]) } else { t.to_string() };
        format!("[{}]: {}", role, clean.replace('\n', " "))
    }).collect()
}

fn format_handover_prompt(sid: &str, label: &str, obj: &str, decs: &[String], prog: &str, excs: &[String]) -> String {
    let dec_str = decs.iter().map(|d| format!("- {}\n", d)).collect::<String>();
    let exc_str = excs.iter().map(|e| format!("{}\n", e)).collect::<String>();
    format!(
r#"[TOPIC CONTEXT HANDOVER]
This session replaces previous session '{sid}' in Telegram forum topic "{label}".
Previous context has been compressed into this handover summary:

### 1. Topic Objective
{obj}

### 2. Key Decisions & Agreed Specifications
{dec_str}
### 3. Current Progress & State
{prog}

### 4. Recent Dialogue Excerpt
{exc_str}
---
INSTRUCTIONS FOR CONTINUING THIS SESSION:
1. Seamlessly continue the work in this topic without losing context.
2. For your very first response:
   - Respond in the language used in the topic dialogue (e.g. Korean if previous dialogue was Korean).
   - Acknowledge that this topic session was refreshed and you have inherited the context.
   - Mention the topic objective and current state in 1-2 concise sentences.
   - Ask the user what next step or task they would like to focus on now."#
    )
}

fn resolve_topic_entries(working_dir: &Path, session_id: &str, past_session_ids: &[String]) -> Vec<TelegramHistoryEntry> {
    let mut entries = extract_topic_history(working_dir, session_id);
    if entries.is_empty() {
        for past_sid in past_session_ids.iter().rev() {
            if past_sid != session_id {
                let past_entries = extract_topic_history(working_dir, past_sid);
                if !past_entries.is_empty() {
                    entries = past_entries;
                    break;
                }
            }
        }
    }
    entries
}

fn resolve_topic_name(working_dir: &Path, session_id: &str, mut topic_name: Option<String>, entries: &[TelegramHistoryEntry]) -> Option<String> {
    if topic_name.is_none() && !session_id.is_empty() {
        let meta_file = working_dir.join("brain").join(session_id).join("topic.json");
        if let Ok(c) = std::fs::read_to_string(&meta_file) {
            if let Ok(m) = serde_json::from_str::<TopicMetadata>(&c) { topic_name = m.topic_name; }
        }
    }
    if topic_name.is_none() {
        topic_name = entries.iter().find_map(|e| e.topic_name.clone().filter(|s| !s.trim().is_empty()));
    }
    topic_name
}

pub fn build_topic_handover(
    working_dir: &Path,
    session_id: &str,
    topic_id: Option<i64>,
    topic_name: Option<String>,
    prior_summary: Option<&str>,
    past_session_ids: &[String],
) -> Option<TopicHandoverContext> {
    let (prior_obj, prior_decs, prior_prog) = prior_summary.map(parse_prior_summary).unwrap_or((None, Vec::new(), None));
    let entries = resolve_topic_entries(working_dir, session_id, past_session_ids);

    if entries.is_empty() && prior_summary.is_none() {
        return None;
    }

    let resolved_name = resolve_topic_name(working_dir, session_id, topic_name, &entries);
    let label = resolved_name.clone().unwrap_or_else(|| topic_id.map(|t| format!("Topic #{}", t)).unwrap_or_else(|| "General Topic".to_string()));
    let obj = extract_topic_objective(&entries, resolved_name.as_deref(), prior_obj.as_deref());
    let decs = extract_agreed_decisions(&entries, &prior_decs);
    let prog = extract_current_progress(&entries, prior_prog.as_deref());
    let excs = format_recent_excerpts(&entries, 4);
    let sid_ref = if !session_id.is_empty() { session_id } else { "prior_session" };
    let prompt = format_handover_prompt(sid_ref, &label, &obj, &decs, &prog, &excs);

    Some(TopicHandoverContext {
        topic_id,
        topic_name: resolved_name,
        previous_session_id: session_id.to_string(),
        topic_objective: obj,
        agreed_decisions: decs,
        current_progress: prog,
        handover_prompt: prompt,
    })
}
