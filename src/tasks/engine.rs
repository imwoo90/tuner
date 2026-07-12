//! Task execution engine
//!
//! Handles running tasks, resolving dependencies, invoking CLI providers, and processing outcomes.

use std::sync::Arc;
use anyhow::{anyhow, Result};

use crate::tasks::models::{TaskResult, TaskEntry};
use crate::tasks::hub::TaskHub;
use crate::tasks::runner::classify_task_response;
use crate::cli::{AgentProvider, CliResponse};

async fn handle_dep_error(hub: &TaskHub, entry: &TaskEntry, err_msg: String) -> Result<()> {
    hub.registry.update_status(&entry.task_id, |e| {
        e.status = "failed".to_string();
        e.error = err_msg.clone();
        e.completed_at = chrono::Utc::now().timestamp() as f64;
    })?;
    let folder = hub.registry.task_folder(&entry.task_id).to_string_lossy().to_string();
    let _ = crate::tasks::runner::deliver_result(hub, crate::tasks::runner::make_err_res(entry, err_msg, folder)).await;
    hub.in_flight.write().await.remove(&entry.task_id);
    Ok(())
}

async fn wait_deps(hub: &TaskHub, entry: &TaskEntry) -> Result<bool> {
    for parent_id in &entry.depends_on {
        loop {
            if let Some(proc_reg) = hub.get_process_registry(&entry.parent_agent).await {
                if proc_reg.is_killed(&entry.task_id).await {
                    return Ok(false);
                }
            }

            let parent_status = hub.registry.get(parent_id).map(|e| e.status);
            match parent_status.as_deref() {
                Some("done") => break,
                Some("failed") | Some("cancelled") => {
                    let err = format!("Dependency parent task {} failed or was cancelled", parent_id);
                    handle_dep_error(hub, entry, err).await?;
                    return Err(anyhow!("Dependency failed"));
                }
                None => {
                    let err = format!("Dependency parent task {} not found", parent_id);
                    handle_dep_error(hub, entry, err).await?;
                    return Err(anyhow!("Dependency not found"));
                }
                _ => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                }
            }
        }
    }
    Ok(true)
}

async fn resolve_cli(hub: &TaskHub, entry: &TaskEntry) -> Result<Arc<dyn AgentProvider>> {
    let guard = hub.cli_services.read().await;
    guard.get(&entry.parent_agent).cloned().or(hub.cli_service.clone())
        .ok_or_else(|| anyhow!("CLIService not available"))
}

fn get_outcome_fields(
    result: Result<CliResponse, String>,
    has_q: bool,
    killed: bool,
) -> (String, String, String, String, i64) {
    match result {
        Ok(resp) => {
            let (status, err) = classify_task_response(
                resp.is_error,
                resp.returncode,
                &resp.result,
                has_q,
            );
            (status, err, resp.session_id.unwrap_or_default(), resp.result, 1i64)
        }
        Err(e) => {
            if killed {
                ("cancelled".to_string(), "".to_string(), "".to_string(), "".to_string(), 0i64)
            } else {
                ("failed".to_string(), e, "".to_string(), "".to_string(), 0i64)
            }
        }
    }
}

fn update_reg_status(
    hub: &TaskHub,
    task_id: &str,
    status: &str,
    elapsed: f64,
    err: &str,
    preview: &str,
    turns: i64,
    sess: &str,
) -> Result<()> {
    hub.registry.update_status(task_id, |e| {
        e.status = status.to_string();
        e.completed_at = chrono::Utc::now().timestamp() as f64;
        e.elapsed_seconds = elapsed;
        e.error = err.to_string();
        e.result_preview = preview.to_string();
        e.num_turns = turns;
        e.session_id = sess.to_string();
    })?;
    Ok(())
}

async fn process_outcome(
    hub: &TaskHub,
    entry: &TaskEntry,
    result: Result<CliResponse, String>,
    elapsed: f64,
    has_q: bool,
) -> Result<TaskResult> {
    let killed = if let Some(proc_reg) = hub.get_process_registry(&entry.parent_agent).await {
        proc_reg.is_killed(&entry.task_id).await
    } else {
        false
    };

    let (status, error_msg, session_id, result_text, num_turns) = get_outcome_fields(result, has_q, killed);
    let total_turns = entry.num_turns + num_turns;
    let mut final_result_text = result_text;

    let preview = if final_result_text.len() > 200 { final_result_text.chars().take(200).collect() } else { final_result_text.clone() };
    update_reg_status(hub, &entry.task_id, &status, elapsed, &error_msg, &preview, total_turns, &session_id)?;

    if status == "done" || status == "cancelled" {
        let taskmemory_path = hub.registry.taskmemory_path(&entry.task_id);
        final_result_text = crate::tasks::runner::append_taskmemory(&final_result_text, &taskmemory_path).await;
    }

    if status == "done" && !session_id.is_empty() {
        final_result_text.push_str(&format!(
            "\n\n---\nTo continue this task's conversation, use:\npython3 tools/task_tools/resume_task.py {} \"your follow-up\"",
            entry.task_id
        ));
    }

    let task_folder = hub.registry.task_folder(&entry.task_id);
    Ok(TaskResult {
        task_id: entry.task_id.clone(),
        chat_id: entry.chat_id,
        parent_agent: entry.parent_agent.clone(),
        name: entry.name.clone(),
        prompt_preview: entry.prompt_preview.clone(),
        result_text: final_result_text,
        status,
        elapsed_seconds: elapsed,
        provider: entry.provider.clone(),
        model: entry.model.clone(),
        session_id,
        error: error_msg,
        task_folder: task_folder.to_string_lossy().to_string(),
        original_prompt: entry.original_prompt.clone(),
        thread_id: entry.thread_id,
    })
}

struct InFlightGuard {
    hub: Arc<TaskHub>,
    task_id: String,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        let hub = self.hub.clone();
        let task_id = self.task_id.clone();
        tokio::spawn(async move {
            let mut inflight = hub.in_flight.write().await;
            let still_running = hub.registry.get(&task_id).map_or(false, |e| e.status == "running");
            let mut outcome_to_deliver = None;
            if still_running {
                let _ = hub.registry.update_status(&task_id, |e| {
                    e.status = "failed".to_string();
                    e.error = "Task execution panicked or aborted".to_string();
                    e.completed_at = chrono::Utc::now().timestamp() as f64;
                });
                if let Some(entry) = hub.registry.get(&task_id) {
                    let folder = hub.registry.task_folder(&task_id).to_string_lossy().to_string();
                    outcome_to_deliver = Some(crate::tasks::runner::make_err_res(&entry, "Task execution panicked or aborted".to_string(), folder));
                }
            }
            inflight.remove(&task_id);
            drop(inflight);

            if let Some(outcome) = outcome_to_deliver {
                let _ = crate::tasks::runner::deliver_result(&hub, outcome).await;
            }
        });
    }
}

/// Standalone function to execute a task and coordinate its lifecycle
pub async fn run_task(
    hub: Arc<TaskHub>,
    task_id: &str,
    prompt: String,
    resume_session: Option<String>,
) -> Result<()> {
    let _guard = InFlightGuard {
        hub: hub.clone(),
        task_id: task_id.to_string(),
    };

    let entry = hub.registry.get(task_id)
        .ok_or_else(|| anyhow!("Task '{}' not found", task_id))?;

    if !wait_deps(&hub, &entry).await? {
        return Ok(());
    }

    let cli = match resolve_cli(&hub, &entry).await {
        Ok(c) => c,
        Err(err) => {
            let err_msg = err.to_string();
            hub.registry.update_status(task_id, |e| {
                e.status = "failed".to_string();
                e.error = err_msg.clone();
                e.completed_at = chrono::Utc::now().timestamp() as f64;
            })?;
            let folder = hub.registry.task_folder(task_id).to_string_lossy().to_string();
            let _ = crate::tasks::runner::deliver_result(&hub, crate::tasks::runner::make_err_res(&entry, err_msg, folder)).await;
            hub.in_flight.write().await.remove(task_id);
            return Ok(());
        }
    };

    let t0 = std::time::Instant::now();
    let task_folder = hub.registry.task_folder(task_id);

    let result = cli.send(
        &prompt,
        resume_session.as_deref(),
        resume_session.is_some(),
        task_folder,
    ).await;

    let elapsed = t0.elapsed().as_secs_f64();
    let has_q = hub.in_flight.read().await.get(task_id).map_or(false, |t| t.has_pending_question);

    let outcome = process_outcome(&hub, &entry, result, elapsed, has_q).await?;
    let _ = crate::tasks::runner::deliver_result(&hub, outcome).await;
    hub.in_flight.write().await.remove(task_id);

    Ok(())
}
