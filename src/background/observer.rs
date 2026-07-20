//! # Background Observer Registry
//!
//! ## Overview
//! Tracks and manages all in-flight background tasks, enforcing execution limits per chat,
//! timing out long-running tasks, and reporting cancellations.
//!
//! ## Collaboration Graph
//! - Spawns asynchronous tasks via tokio join handles.
//! - Interacts with [`MessageBus`](crate::bus::bus::MessageBus) to broadcast task output.
//!
//! ## Search Tags
//! #task-registry, #drop-guard, #concurrency-limits, #timeout-tracker

pub use super::models::{BackgroundResult, BackgroundResultStatus, BackgroundSubmit};
use crate::cli::AgentProvider;
use crate::workspace::paths::DuctorPaths;
use crate::config::CliConfig;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

/// Maximum number of active tasks allowed per Telegram chat.
pub const MAX_TASKS_PER_CHAT: usize = 5;

pub type ResultHandler = Arc<dyn Fn(BackgroundResult) + Send + Sync>;

/// Holds state for an in-flight background task.
pub struct BackgroundTask {
    pub chat_id: i64,
    pub join_handle: Option<JoinHandle<()>>,
}

/// Coordinates task submission, execution timeouts, cancellation and callbacks.
#[derive(Clone)]
pub struct BackgroundObserver {
    inner: Arc<BackgroundObserverInner>,
}

struct BackgroundObserverInner {
    paths: DuctorPaths,
    timeout: Duration,
    on_result: Mutex<Option<ResultHandler>>,
    tasks: Mutex<HashMap<String, BackgroundTask>>,
}

struct TaskGuard {
    task_id: String,
    inner: Arc<BackgroundObserverInner>,
    chat_id: i64,
    message_id: i64,
    thread_id: Option<i64>,
    prompt: String,
    start_time: Instant,
    completed: bool,
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        if !self.completed {
            let elapsed = self.start_time.elapsed().as_secs_f64();
            let result = BackgroundResult {
                task_id: self.task_id.clone(),
                chat_id: self.chat_id,
                message_id: self.message_id,
                thread_id: self.thread_id,
                prompt_preview: if self.prompt.len() > 60 {
                    format!("{}...", &self.prompt[..57])
                } else {
                    self.prompt.clone()
                },
                result_text: String::new(),
                status: BackgroundResultStatus::Aborted,
                elapsed_seconds: elapsed,
            };
            if let Some(ref handler) = *self.inner.on_result.lock().unwrap() {
                handler(result);
            }
            self.inner.tasks.lock().unwrap().remove(&self.task_id);
        }
    }
}

impl BackgroundObserver {
    /// Create a new BackgroundObserver instance.
    pub fn new(paths: DuctorPaths, timeout: Duration) -> Self {
        Self {
            inner: Arc::new(BackgroundObserverInner {
                paths,
                timeout,
                on_result: Mutex::new(None),
                tasks: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Register a callback to handle task completion results.
    pub async fn set_result_handler<F>(&self, handler: F)
    where
        F: Fn(BackgroundResult) + Send + Sync + 'static,
    {
        let mut lock = self.inner.on_result.lock().unwrap();
        *lock = Some(Arc::new(handler));
    }

    /// Submit a task to be run in the background.
    pub async fn submit(
        &self,
        provider: Arc<dyn AgentProvider>,
        submit: BackgroundSubmit,
        _config: CliConfig,
    ) -> Result<String, String> {
        let mut tasks = self.inner.tasks.lock().unwrap();
        let mut active = 0;
        for task in tasks.values() {
            if task.chat_id == submit.chat_id {
                if let Some(ref handle) = task.join_handle {
                    if !handle.is_finished() {
                        active += 1;
                    }
                }
            }
        }
        if active >= MAX_TASKS_PER_CHAT {
            return Err("Too many active tasks".to_string());
        }

        let task_id = generate_task_id();
        let inner_clone = self.inner.clone();
        let task_id_clone = task_id.clone();
        let submit_clone = submit.clone();

        let join_handle = tokio::spawn(async move {
            inner_clone.run_task(&task_id_clone, provider, submit_clone).await;
        });

        tasks.insert(task_id.clone(), BackgroundTask {
            chat_id: submit.chat_id,
            join_handle: Some(join_handle),
        });

        Ok(task_id)
    }

    /// List active task IDs for a given chat, or all chats if None.
    pub async fn active_tasks(&self, chat_id: Option<i64>) -> Vec<String> {
        let tasks = self.inner.tasks.lock().unwrap();
        let mut active = Vec::new();
        for (task_id, task) in tasks.iter() {
            if let Some(ref handle) = task.join_handle {
                if !handle.is_finished() {
                    if chat_id.is_none() || chat_id == Some(task.chat_id) {
                        active.push(task_id.clone());
                    }
                }
            }
        }
        active
    }

    /// Cancel all background tasks for a given chat ID.
    pub async fn cancel_all(&self, chat_id: i64) -> usize {
        let mut tasks = self.inner.tasks.lock().unwrap();
        let mut to_abort = Vec::new();
        for (task_id, task) in tasks.iter() {
            if task.chat_id == chat_id {
                if let Some(ref handle) = task.join_handle {
                    handle.abort();
                    to_abort.push(task_id.clone());
                }
            }
        }
        let count = to_abort.len();
        let mut handles = Vec::new();
        for id in &to_abort {
            if let Some(mut task) = tasks.remove(id) {
                if let Some(handle) = task.join_handle.take() {
                    handles.push(handle);
                }
            }
        }
        drop(tasks);
        for handle in handles {
            let _ = handle.await;
        }
        count
    }

    /// Abort all outstanding background tasks.
    pub async fn shutdown(&self) {
        let mut tasks = self.inner.tasks.lock().unwrap();
        let mut handles = Vec::new();
        for (_, mut task) in tasks.drain() {
            if let Some(handle) = task.join_handle.take() {
                handle.abort();
                handles.push(handle);
            }
        }
        drop(tasks);
        for handle in handles {
            let _ = handle.await;
        }
    }
}

impl BackgroundObserverInner {
    fn map_run_result(
        &self,
        run_res: Result<Result<crate::cli::CliResponse, String>, tokio::time::error::Elapsed>,
    ) -> (BackgroundResultStatus, String) {
        match run_res {
            Ok(Ok(response)) => {
                let status = if response.is_error {
                    if response.result.contains("not found") || response.stderr.contains("not found") {
                        BackgroundResultStatus::ErrorCliNotFound
                    } else {
                        BackgroundResultStatus::ErrorCli
                    }
                } else {
                    BackgroundResultStatus::Success
                };
                (status, response.result)
            }
            Ok(Err(err)) => {
                let status = if err.contains("not found") {
                    BackgroundResultStatus::ErrorCliNotFound
                } else {
                    BackgroundResultStatus::ErrorInternal
                };
                (status, err)
            }
            Err(_) => {
                (BackgroundResultStatus::ErrorTimeout, "Background task timed out".to_string())
            }
        }
    }

    async fn run_task(
        self: Arc<Self>,
        task_id: &str,
        provider: Arc<dyn AgentProvider>,
        submit: BackgroundSubmit,
    ) {
        let guard = TaskGuard {
            task_id: task_id.to_string(),
            inner: self.clone(),
            chat_id: submit.chat_id,
            message_id: submit.message_id,
            thread_id: submit.thread_id,
            prompt: submit.prompt.clone(),
            start_time: Instant::now(),
            completed: false,
        };

        let workspace_path = self.paths.workspace();
        let run_res = tokio::time::timeout(
            self.timeout,
            provider.send(&submit.prompt, None, false, workspace_path),
        ).await;

        let (status, result_text) = self.map_run_result(run_res);

        let elapsed = guard.start_time.elapsed().as_secs_f64();
        let result_delivery = BackgroundResult {
            task_id: guard.task_id.clone(),
            chat_id: guard.chat_id,
            message_id: guard.message_id,
            thread_id: guard.thread_id,
            prompt_preview: if guard.prompt.len() > 60 {
                format!("{}...", &guard.prompt[..57])
            } else {
                guard.prompt.clone()
            },
            result_text,
            status,
            elapsed_seconds: elapsed,
        };

        let mut guard_mut = guard;
        guard_mut.completed = true;

        if let Some(ref handler) = *self.on_result.lock().unwrap() {
            handler(result_delivery);
        }

        self.tasks.lock().unwrap().remove(task_id);
    }
}

fn generate_task_id() -> String {
    use std::io::Read;
    let mut seed = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u32;
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let mut buf = [0u8; 4];
        if f.read_exact(&mut buf).is_ok() {
            seed = u32::from_ne_bytes(buf);
        }
    }
    format!("{:08x}", seed)
}
