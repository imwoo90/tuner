//! # Antigravity Transcript Background Task Tracker
//!
//! ## Overview
//! Replays `CompactStep` records in strict monotonic `step_index` order (`P + 1 + k`
//! tool correlation) to track live one-shot `schedule` timers, recurring non-daemon
//! `Cron:` sentinels, and non-daemon background tasks while filtering `IsDaemon` jobs.
//!
//! ## Search Tags
//! #task-tracker, #step-index-order, #schedule-timer, #daemon-filter

pub use super::step_parser::parse_timer_desc_secs;
use super::step_parser::{CompactStep, PendingCall};
use std::collections::HashMap;

fn extract_task_id(content: &str) -> Option<String> {
    content
        .split("task id:")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .filter(|t| t.contains("/task-"))
        .map(|tok| tok.trim_matches(|c: char| c == ',' || c == '.').to_string())
}

struct TaskEntry {
    deadline: u64,
    cond: String,
    is_cron: bool,
}

#[derive(Default)]
pub(crate) struct TaskTracker {
    tasks: HashMap<String, TaskEntry>,
    call_map: HashMap<(u8, u64), PendingCall>,
}

impl TaskTracker {
    pub(crate) fn has_live_task(&self, now_epoch: u64) -> bool {
        self.tasks.values().any(|t| now_epoch < t.deadline)
    }

    pub(crate) fn ingest_ordered_step(&mut self, key: (u8, u64), step: &CompactStep) {
        if step.stype == "PLANNER_RESPONSE" {
            for (k, call) in step.calls.iter().enumerate() {
                if let PendingCall::ManageTask {
                    kill_tid: Some(tid),
                } = call
                {
                    self.tasks.remove(tid);
                }
                self.call_map
                    .insert((key.0, key.1.saturating_add(1 + k as u64)), call.clone());
            }
        } else if step.stype == "GENERIC" {
            let call = self.call_map.remove(&key);
            if let Some(ref c) = step.content {
                if step.status == "RUNNING" {
                    self.record_running_generic(call, c, step.step_ts);
                } else if step.status == "DONE"
                    && matches!(call, Some(PendingCall::ManageTask { .. }))
                {
                    self.ingest_manage_task_done(c);
                }
            }
        } else if step.stype == "SYSTEM_MESSAGE"
            && let Some(ref c) = step.content
        {
            self.ingest_system_message(c, step.step_ts);
        }
    }

    fn record_running_generic(&mut self, call: Option<PendingCall>, content: &str, step_ts: u64) {
        let Some(tid) = extract_task_id(content) else {
            return;
        };
        if content.contains("Task Description: Timer:") {
            let (q_dur, cond, is_d) = match call {
                Some(PendingCall::Schedule { dur, cond, is_daemon }) => (dur, cond, is_daemon),
                _ => (None, "never".into(), false),
            };
            let dur = q_dur.unwrap_or(0).max(parse_timer_desc_secs(content).unwrap_or(0));
            if !is_d && dur > 0 {
                let deadline = step_ts.saturating_add(dur).saturating_add(60);
                self.tasks.insert(tid, TaskEntry { deadline, cond, is_cron: false });
            }
        } else if content.contains("Task Description: Cron:") {
            if !matches!(call, Some(PendingCall::Schedule { is_daemon: true, .. })) {
                let deadline = step_ts.saturating_add(3900);
                self.tasks.insert(tid, TaskEntry { deadline, cond: "never".into(), is_cron: true });
            }
        } else if !matches!(call, Some(PendingCall::RunCommand { is_daemon: true })) {
            let deadline = step_ts.saturating_add(900);
            self.tasks.insert(tid, TaskEntry { deadline, cond: "never".into(), is_cron: false });
        }
    }

    fn ingest_manage_task_done(&mut self, content: &str) {
        if content.contains("No background tasks are currently running.") {
            self.tasks.clear();
        } else if content.contains("background task(s) currently running:")
            && !content.contains("<truncated ")
        {
            self.tasks.retain(|tid, _| content.contains(tid.as_str()));
        }
        for seg in content.split("Task \"").skip(1) {
            if let Some(tid) = seg
                .split("\" cancelled.")
                .next()
                .filter(|t| t.contains("/task-"))
            {
                self.tasks.remove(tid);
            }
        }
        if let Some(tid) = content
            .split("Task:")
            .nth(1)
            .and_then(|r| r.split_whitespace().next())
            && let Some(st_tok) = content
                .split("Status:")
                .nth(1)
                .and_then(|r| r.split_whitespace().next())
            && st_tok != "RUNNING"
        {
            self.tasks.remove(tid);
        }
    }

    fn ingest_system_message(&mut self, content: &str, step_ts: u64) {
        for ln in content.lines() {
            let trimmed = ln.trim_start();
            if !trimmed.starts_with("[Message] ") {
                continue;
            }
            let (hdr, body) = match trimmed.split_once(" content=") {
                Some((h, b)) => (h, b),
                None => (trimmed, ""),
            };
            let Some(snd) = hdr
                .split("sender=")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .map(|s| s.trim_matches(|c: char| c == ',' || c == '>' || c == ']'))
            else {
                continue;
            };
            if snd == "system"
                && body.starts_with(
                    "[Notice] All your subagents and background tasks have been stopped due to server restart",
                )
            {
                self.tasks.clear();
                continue;
            }
            if snd == "system" {
                continue;
            }
            if let Some(entry) = self.tasks.get_mut(snd)
                && entry.is_cron
            {
                entry.deadline = step_ts.saturating_add(3900);
            } else {
                self.tasks.remove(snd);
            }
            self.tasks
                .retain(|_, t| t.is_cron || (t.cond != "any" && t.cond != snd));
        }
    }
}
