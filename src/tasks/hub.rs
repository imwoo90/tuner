//! # Task Callback and Interactivity Hub
//!
//! ## Overview
//! Manages bidirectional communication with executing tasks. Routes interactive answers back to tasks.
//!
//! ## Collaboration Graph
//! - Used by [`CronScheduler`](crate::cron::scheduler::CronScheduler) and Telegram callback routers.
//!
//! ## Search Tags
//! #callback-hub, #interactivity-manager, #stdout-channels

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use anyhow::Result;
use async_trait::async_trait;

use crate::tasks::models::{TaskEntry, TaskSubmit, TaskResult, TasksConfig};
use crate::tasks::registry::TaskRegistry;
use crate::tasks::runner::ProcessRegistry;
use crate::cli::AgentProvider;

/// A callback invoked when a background task completes
#[async_trait]
pub trait TaskResultCallback: Send + Sync {
    async fn call(&self, result: TaskResult);
}

/// A handler for forwarding questions from the task agent
#[async_trait]
pub trait QuestionHandler: Send + Sync {
    async fn call(
        &self,
        task_id: &str,
        question: &str,
        prompt_preview: &str,
        chat_id: i64,
        thread_id: Option<i64>,
    ) -> Result<String, String>;
}

/// Struct representing an in-flight task
pub struct TaskInFlight {
    pub entry: TaskEntry,
    pub join_handle: Option<tokio::task::JoinHandle<()>>,
    pub has_pending_question: bool,
}

/// Central coordinator for task execution, lifecycle, and delegation
pub struct TaskHub {
    pub registry: Arc<TaskRegistry>,
    pub config: TasksConfig,
    pub cli_service: Option<Arc<dyn AgentProvider>>,
    pub cli_services: Arc<RwLock<HashMap<String, Arc<dyn AgentProvider>>>>,
    pub agent_tasks_dirs: Arc<RwLock<HashMap<String, PathBuf>>>,
    pub agent_chat_ids: Arc<RwLock<HashMap<String, i64>>>,
    pub process_registry: Option<Arc<ProcessRegistry>>,
    pub agent_process_registries: Arc<RwLock<HashMap<String, Arc<ProcessRegistry>>>>,
    pub in_flight: Arc<RwLock<HashMap<String, TaskInFlight>>>,
    pub result_handlers: Arc<RwLock<HashMap<String, Arc<dyn TaskResultCallback>>>>,
    pub question_handlers: Arc<RwLock<HashMap<String, Arc<dyn QuestionHandler>>>>,
    maintenance_task: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
}

impl TaskHub {
    /// Create a new TaskHub instance
    pub fn new(
        registry: Arc<TaskRegistry>,
        config: TasksConfig,
        cli_service: Option<Arc<dyn AgentProvider>>,
        process_registry: Option<Arc<ProcessRegistry>>,
    ) -> Self {
        let pr = process_registry.clone().unwrap_or_else(|| Arc::new(ProcessRegistry::new()));
        let _ = crate::tasks::runner::GLOBAL_PROCESS_REGISTRY.set(pr);
        Self {
            registry,
            config,
            cli_service,
            cli_services: Arc::new(RwLock::new(HashMap::new())),
            agent_tasks_dirs: Arc::new(RwLock::new(HashMap::new())),
            agent_chat_ids: Arc::new(RwLock::new(HashMap::new())),
            process_registry,
            agent_process_registries: Arc::new(RwLock::new(HashMap::new())),
            in_flight: Arc::new(RwLock::new(HashMap::new())),
            result_handlers: Arc::new(RwLock::new(HashMap::new())),
            question_handlers: Arc::new(RwLock::new(HashMap::new())),
            maintenance_task: Arc::new(RwLock::new(None)),
        }
    }

    /// Sets the result handler for an agent
    pub async fn set_result_handler(&self, agent_name: &str, handler: Arc<dyn TaskResultCallback>) {
        self.result_handlers.write().await.insert(agent_name.to_string(), handler);
    }

    /// Sets the question handler for an agent
    pub async fn set_question_handler(&self, agent_name: &str, handler: Arc<dyn QuestionHandler>) {
        self.question_handlers.write().await.insert(agent_name.to_string(), handler);
    }

    /// Sets a per-agent CLI service
    pub async fn set_cli_service(&self, agent_name: &str, cli: Arc<dyn AgentProvider>) {
        self.cli_services.write().await.insert(agent_name.to_string(), cli);
    }

    /// Sets a per-agent process registry
    pub async fn set_agent_process_registry(&self, agent_name: &str, process_registry: Arc<ProcessRegistry>) {
        self.agent_process_registries.write().await.insert(agent_name.to_string(), process_registry);
    }

    /// Sets agent tasks directory path override
    pub async fn set_agent_paths(&self, agent_name: &str, tasks_dir: PathBuf) {
        self.agent_tasks_dirs.write().await.insert(agent_name.to_string(), tasks_dir);
    }

    /// Sets agent chat ID mapping
    pub async fn set_agent_chat_id(&self, agent_name: &str, chat_id: i64) {
        self.agent_chat_ids.write().await.insert(agent_name.to_string(), chat_id);
    }

    /// Start periodic maintenance loop
    pub async fn start_maintenance(&self) {
        let mut mt = self.maintenance_task.write().await;
        if mt.is_none() {
            let registry = self.registry.clone();
            let handle = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(5 * 3600)).await;
                    let _ = registry.cleanup_orphans();
                }
            });
            *mt = Some(handle);
        }
    }

    /// Resolves the process registry for a given parent agent
    pub async fn get_process_registry(&self, parent_agent: &str) -> Option<Arc<ProcessRegistry>> {
        let guard = self.agent_process_registries.read().await;
        if let Some(r) = guard.get(parent_agent) {
            Some(r.clone())
        } else {
            self.process_registry.clone()
        }
    }

    /// Submit a new task for execution
    pub async fn submit(self: &Arc<Self>, submit: TaskSubmit) -> Result<String> {
        crate::tasks::manager::submit_task(self, submit).await
    }

    /// Resumes a task from a waiting/failed/cancelled status with follow up prompt
    pub async fn resume(self: &Arc<Self>, task_id: &str, follow_up: &str) -> Result<String> {
        crate::tasks::manager::resume_task(self, task_id, follow_up).await
    }

    /// Forward a task agent's question to the parent agent
    pub async fn forward_question(self: &Arc<Self>, task_id: &str, question: &str) -> Result<String> {
        crate::tasks::manager::forward_question(self, task_id, question).await
    }

    /// Cancel a running task
    pub async fn cancel(&self, task_id: &str) -> bool {
        let mut inflight = self.in_flight.write().await;
        if let Some(mut t) = inflight.remove(task_id) {
            if let Some(proc_reg) = self.get_process_registry(&t.entry.parent_agent).await {
                let _ = proc_reg.kill_for_task(task_id).await;
            }

            if let Some(handle) = t.join_handle.take() {
                handle.abort();
                let _ = handle.await;
            }

            let _ = self.registry.update_status(task_id, |e| {
                e.status = "cancelled".to_string();
                e.completed_at = chrono::Utc::now().timestamp() as f64;
            });

            let taskmemory_path = self.registry.taskmemory_path(task_id);
            let mut partial_text = String::new();
            if taskmemory_path.is_file() {
                if let Ok(content) = std::fs::read_to_string(&taskmemory_path) {
                    partial_text = content;
                }
            }

            let _ = self.deliver_cancelled_result(&t.entry, partial_text).await;
            true
        } else {
            false
        }
    }

    async fn deliver_cancelled_result(&self, entry: &TaskEntry, partial_text: String) -> Result<()> {
        let task_folder = self.registry.task_folder(&entry.task_id);
        let outcome = TaskResult {
            task_id: entry.task_id.clone(),
            chat_id: entry.chat_id,
            parent_agent: entry.parent_agent.clone(),
            name: entry.name.clone(),
            prompt_preview: entry.prompt_preview.clone(),
            result_text: partial_text,
            status: "cancelled".to_string(),
            elapsed_seconds: 0.0,
            provider: entry.provider.clone(),
            model: entry.model.clone(),
            session_id: entry.session_id.clone(),
            error: "".to_string(),
            task_folder: task_folder.to_string_lossy().to_string(),
            original_prompt: entry.original_prompt.clone(),
            thread_id: entry.thread_id,
        };
        let handler = {
            let guard = self.result_handlers.read().await;
            guard.get(&entry.parent_agent).cloned()
        };
        if let Some(h) = handler {
            h.call(outcome).await;
        }
        Ok(())
    }

    /// Cancels all running tasks for a chat
    pub async fn cancel_all(&self, chat_id: i64) -> usize {
        let targets = {
            let inflight = self.in_flight.read().await;
            inflight.values()
                .filter(|t| t.entry.chat_id == chat_id)
                .map(|t| (t.entry.task_id.clone(), t.entry.parent_agent.clone()))
                .collect::<Vec<_>>()
        };

        let mut count = 0;
        for (tid, _) in &targets {
            if self.cancel(tid).await {
                count += 1;
            }
        }
        count
    }

    /// Lists active task entries
    pub async fn active_tasks(&self, chat_id: Option<i64>) -> Vec<TaskEntry> {
        let inflight = self.in_flight.read().await;
        inflight.values()
            .filter(|t| chat_id.map_or(true, |c| t.entry.chat_id == c))
            .map(|t| t.entry.clone())
            .collect()
    }

    /// Gracefully shutdown the task hub
    pub async fn shutdown(&self) {
        if let Some(handle) = self.maintenance_task.write().await.take() {
            handle.abort();
        }

        let mut inflight = self.in_flight.write().await;
        for (_, mut t) in inflight.drain() {
            if let Some(proc_reg) = self.get_process_registry(&t.entry.parent_agent).await {
                let _ = proc_reg.kill_for_task(&t.entry.task_id).await;
            }
            if let Some(handle) = t.join_handle.take() {
                handle.abort();
            }
        }
    }
}
