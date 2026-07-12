//! Task manager operations
//!
//! Implements submission, resumption, and question forwarding for TaskHub.

use std::sync::Arc;
use anyhow::{anyhow, Result};

use crate::tasks::models::{TaskSubmit, TaskEntry, normalise_priority};
use crate::tasks::hub::{TaskHub, TaskInFlight};

const TASK_PROMPT_SUFFIX: &str = r#"

---
TASK RULES (MANDATORY):
You are a background task agent. You have NO direct user access.

IMPORTANT — If you need ANY information to complete this task (missing details,
clarifications, preferences), you MUST use this tool:
```
python3 tools/task_tools/ask_parent.py "your question here"
```
Do NOT include questions in your response text. The tool forwards your question
to the parent agent who will resume you with the answer.

After finishing, update your task memory: {taskmemory_path}
"#;



fn check_resumable(e: &TaskEntry) -> Result<()> {
    let r = ["done", "failed", "cancelled", "waiting"];
    if !r.contains(&e.status.as_str()) {
        return Err(anyhow!("Task '{}' is still {}", e.task_id, e.status));
    }
    if e.session_id.is_empty() {
        return Err(anyhow!("Task '{}' has no resumable session", e.task_id));
    }
    if e.provider.is_empty() {
        return Err(anyhow!("Task '{}' has no provider recorded", e.task_id));
    }
    Ok(())
}

/// Submits a task to the coordinator
pub async fn submit_task(hub: &Arc<TaskHub>, mut submit: TaskSubmit) -> Result<String> {
    if !hub.config.enabled {
        return Err(anyhow!("Task system is disabled"));
    }

    if submit.chat_id == 0 {
        if let Some(&resolved) = hub.agent_chat_ids.read().await.get(&submit.parent_agent) {
            submit.chat_id = resolved;
        }
    }

    let priority = normalise_priority(Some(&submit.priority));

    let mut inflight = hub.in_flight.write().await;

    if priority != "interactive" {
        let active = inflight.values().filter(|t| {
            t.entry.chat_id == submit.chat_id && t.entry.priority != "interactive"
        }).count();
        if active >= hub.config.max_parallel {
            return Err(anyhow!("Too many background tasks ({} max)", hub.config.max_parallel));
        }
    }

    let p = submit.provider_override.clone();
    let m = submit.model_override.clone();
    let th = submit.thinking_override.clone();
    let agent_tasks_dir = hub.agent_tasks_dirs.read().await.get(&submit.parent_agent).cloned();

    let entry = hub.registry.create(
        submit.clone(), p, m, th, agent_tasks_dir, Some(priority),
    )?;

    let task_id = entry.task_id.clone();
    let task_id_clone = task_id.clone();
    let hub_clone = hub.clone();

    let taskmemory_path = hub.registry.taskmemory_path(&task_id);
    let full_prompt = format!("{}{}", submit.prompt, TASK_PROMPT_SUFFIX.replace("{taskmemory_path}", &taskmemory_path.to_string_lossy()));

    let join_handle = tokio::spawn(async move {
        let _ = crate::tasks::engine::run_task(hub_clone, &task_id_clone, full_prompt, None).await;
    });

    inflight.insert(task_id.clone(), TaskInFlight {
        entry,
        join_handle: Some(join_handle),
        has_pending_question: false,
    });

    Ok(task_id)
}

fn make_reminder_prompt(follow_up: &str, taskmemory_path: &std::path::Path) -> String {
    let reminder = format!(
        "\n\n---\nREMINDER: You are a background task agent with NO direct user access.\n- Need more info? Use: python3 tools/task_tools/ask_parent.py \"question\"\n- Do NOT put questions in your response — the user cannot see them.\n- When done, write your final results to: {}",
        taskmemory_path.to_string_lossy()
    );
    format!("{}{}", follow_up, reminder)
}

fn update_inflight_entry(
    inflight: &mut std::collections::HashMap<String, TaskInFlight>,
    task_id: &str,
    entry: &TaskEntry,
    join_handle: tokio::task::JoinHandle<()>,
) {
    if let Some(t) = inflight.get_mut(task_id) {
        t.entry.status = "running".to_string();
        t.entry.completed_at = 0.0;
        t.entry.error.clear();
        t.entry.result_preview.clear();
        t.entry.last_question.clear();
        t.join_handle = Some(join_handle);
        t.has_pending_question = false;
    } else {
        let mut entry_running = entry.clone();
        entry_running.status = "running".to_string();
        entry_running.completed_at = 0.0;
        entry_running.error.clear();
        entry_running.result_preview.clear();
        entry_running.last_question.clear();
        inflight.insert(task_id.to_string(), TaskInFlight {
            entry: entry_running,
            join_handle: Some(join_handle),
            has_pending_question: false,
        });
    }
}

/// Resumes a waiting/cancelled/failed task
pub async fn resume_task(hub: &Arc<TaskHub>, task_id: &str, follow_up: &str) -> Result<String> {
    if !hub.config.enabled {
        return Err(anyhow!("Task system is disabled"));
    }

    let entry = hub.registry.get(task_id)
        .ok_or_else(|| anyhow!("Task '{}' not found", task_id))?;

    check_resumable(&entry)?;

    let mut inflight = hub.in_flight.write().await;

    if entry.priority != "interactive" {
        let active = inflight.values().filter(|t| {
            t.entry.chat_id == entry.chat_id && t.entry.priority != "interactive"
        }).count();
        if active >= hub.config.max_parallel {
            return Err(anyhow!("Too many background tasks ({} max)", hub.config.max_parallel));
        }
    }

    hub.registry.update_status(task_id, |e| {
        e.status = "running".to_string();
        e.completed_at = 0.0;
        e.error.clear();
        e.result_preview.clear();
        e.last_question.clear();
    })?;

    let taskmemory_path = hub.registry.taskmemory_path(task_id);
    let full_prompt = make_reminder_prompt(follow_up, &taskmemory_path);

    let hub_clone = hub.clone();
    let task_id_clone = task_id.to_string();
    let session_id_clone = entry.session_id.clone();
    let join_handle = tokio::spawn(async move {
        let _ = crate::tasks::engine::run_task(hub_clone, &task_id_clone, full_prompt, Some(session_id_clone)).await;
    });

    update_inflight_entry(&mut inflight, task_id, &entry, join_handle);

    Ok(task_id.to_string())
}

/// Forwards a question to the parent agent
pub async fn forward_question(hub: &Arc<TaskHub>, task_id: &str, question: &str) -> Result<String> {
    let entry = match hub.registry.get(task_id) {
        Some(e) => e,
        None => return Ok("Error: Task not found".to_string()),
    };

    let handler = {
        let guard = hub.question_handlers.read().await;
        match guard.get(&entry.parent_agent).cloned() {
            Some(h) => h,
            None => return Ok(format!("Error: No question handler for agent '{}'", entry.parent_agent)),
        }
    };

    let q_count = entry.question_count + 1;
    let last_q = if question.chars().count() > 200 {
        question.chars().take(200).collect::<String>()
    } else {
        question.to_string()
    };
    hub.registry.update_status(task_id, |e| {
        e.question_count = q_count;
        e.last_question = last_q.to_string();
    })?;

    if let Some(t) = hub.in_flight.write().await.get_mut(task_id) {
        t.has_pending_question = true;
    }

    let task_id_clone = task_id.to_string();
    let question_clone = question.to_string();
    let prompt_preview_clone = entry.prompt_preview.clone();
    let chat_id = entry.chat_id;
    let thread_id = entry.thread_id;

    tokio::spawn(async move {
        let _ = handler.call(&task_id_clone, &question_clone, &prompt_preview_clone, chat_id, thread_id).await;
    });

    Ok("Question forwarded to parent agent. Finish your current work — you will be resumed with the answer.".to_string())
}
