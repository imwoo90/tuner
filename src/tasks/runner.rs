//! Task subprocess runner and process registry
//!
//! Manages active subprocess registrations, signaling, and exit code classification.

//! 
//! ## Search Tags
//! #runner

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Tracks active processes and maps task labels to process identifiers
pub struct ProcessRegistry {
    pids: Arc<Mutex<HashMap<String, u32>>>,
    killed_tasks: Arc<Mutex<HashSet<String>>>,
}

impl ProcessRegistry {
    /// Creates a new ProcessRegistry instance
    pub fn new() -> Self {
        Self {
            pids: Arc::new(Mutex::new(HashMap::new())),
            killed_tasks: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Registers a process ID under a task_id
    pub async fn register(&self, task_id: String, pid: u32) {
        self.pids.lock().await.insert(task_id, pid);
    }

    /// Unregisters a task_id
    pub async fn unregister(&self, task_id: &str) {
        self.pids.lock().await.remove(task_id);
    }

    /// Kills the process tree associated with a task_id
    pub async fn kill_for_task(&self, task_id: &str) -> bool {
        self.killed_tasks.lock().await.insert(task_id.to_string());
        let mut pids = self.pids.lock().await;
        if let Some(pid) = pids.remove(task_id) {
            let pgid = nix::unistd::Pid::from_raw(-(pid as i32));
            let _ = nix::sys::signal::kill(pgid, nix::sys::signal::Signal::SIGTERM);
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let _ = nix::sys::signal::kill(pgid, nix::sys::signal::Signal::SIGKILL);
            let direct_pid = nix::unistd::Pid::from_raw(pid as i32);
            let _ = nix::sys::signal::kill(direct_pid, nix::sys::signal::Signal::SIGKILL);
            true
        } else {
            true
        }
    }

    /// Checks if a task has been marked as killed
    pub async fn is_killed(&self, task_id: &str) -> bool {
        self.killed_tasks.lock().await.contains(task_id)
    }
}

impl Default for ProcessRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub static GLOBAL_PROCESS_REGISTRY: std::sync::OnceLock<Arc<ProcessRegistry>> = std::sync::OnceLock::new();

/// Classifies a CLI outcome to status and error message
pub fn classify_task_response(
    is_error: bool,
    returncode: Option<i32>,
    result: &str,
    has_pending_question: bool,
) -> (String, String) {
    if is_error {
        if let Some(code) = returncode {
            if code == 143 || code == 137 || code == -15 || code == -9 {
                return ("cancelled".to_string(), "".to_string());
            }
        }
        return (
            "failed".to_string(),
            if result.is_empty() { "CLI error".to_string() } else { result.to_string() },
        );
    }
    if has_pending_question {
        return ("waiting".to_string(), "".to_string());
    }
    ("done".to_string(), "".to_string())
}

use crate::tasks::models::{TaskResult, TaskEntry};
use crate::tasks::hub::TaskHub;
use std::path::Path;

pub fn make_err_res(e: &TaskEntry, err: String, folder: String) -> TaskResult {
    TaskResult {
        task_id: e.task_id.clone(),
        chat_id: e.chat_id,
        parent_agent: e.parent_agent.clone(),
        name: e.name.clone(),
        prompt_preview: e.prompt_preview.clone(),
        result_text: "".to_string(),
        status: "failed".to_string(),
        elapsed_seconds: 0.0,
        provider: e.provider.clone(),
        model: e.model.clone(),
        session_id: "".to_string(),
        error: err,
        task_folder: folder,
        original_prompt: e.original_prompt.clone(),
        thread_id: e.thread_id,
    }
}

pub async fn deliver_result(hub: &TaskHub, result: TaskResult) -> anyhow::Result<()> {
    let handler = {
        let guard = hub.result_handlers.read().await;
        guard.get(&result.parent_agent).cloned()
    };
    if let Some(h) = handler {
        h.call(result).await;
    }
    Ok(())
}

pub async fn append_taskmemory(result_text: &str, taskmemory_path: &Path) -> String {
    if !taskmemory_path.is_file() {
        return result_text.to_string();
    }

    if let Ok(content) = tokio::fs::read_to_string(taskmemory_path).await {
        let content_trimmed = content.trim();
        if content_trimmed.is_empty() {
            return result_text.to_string();
        }

        const MAX_LEN: usize = 4000;
        let char_count = content_trimmed.chars().count();
        if char_count > MAX_LEN {
            eprintln!(
                "WARNING: TASKMEMORY truncated at {:?}: {} chars -> {} chars",
                taskmemory_path, char_count, MAX_LEN
            );

            let truncated = content_trimmed.chars().take(MAX_LEN).collect::<String>();
            let suffix = format!(
                "\n[... truncated -- original was {} chars. Full content at: {}]",
                char_count,
                taskmemory_path.display()
            );

            return format!(
                "{}\n\n---\nCONTENT FROM TASKMEMORY.MD ({:?}):\n\n{}{}",
                result_text, taskmemory_path, truncated, suffix
            );
        } else {
            return format!(
                "{}\n\n---\nCONTENT FROM TASKMEMORY.MD ({:?}):\n\n{}",
                result_text, taskmemory_path, content_trimmed
            );
        }
    }
    result_text.to_string()
}

