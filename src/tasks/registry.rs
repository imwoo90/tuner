//! Task registry persistent CRUD
//!
//! Implements atomic JSON load/save operations, stale task state recovery,
//! and seeding task workspaces with markdown configs.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use anyhow::{anyhow, Context, Result};

use crate::tasks::models::{TaskEntry, TaskSubmit, normalise_priority};
use crate::tasks::dag::check_cycle;

const PROMPT_PREVIEW_LEN: usize = 80;

#[derive(serde::Serialize, serde::Deserialize)]
struct RegistryData {
    tasks: Vec<TaskEntry>,
}

/// Persistent registry for background tasks metadata
pub struct TaskRegistry {
    pub(crate) registry_path: PathBuf,
    pub(crate) default_tasks_dir: PathBuf,
    pub(crate) entries: Mutex<HashMap<String, TaskEntry>>,
}

fn seed_folder_helper(resolved_dir: &Path, task_id: &str, entry: &TaskEntry, prompt: &str) -> Result<()> {
    let folder = resolved_dir.join(task_id);
    fs::create_dir_all(&folder)?;
    seed_task_folder(&folder, entry, prompt)?;
    Ok(())
}

fn get_adj(entries: &HashMap<String, TaskEntry>, task_id: String, submit: &TaskSubmit) -> HashMap<String, Vec<String>> {
    let mut adj = HashMap::new();
    for (tid, entry) in entries.iter() {
        adj.insert(tid.clone(), entry.depends_on.clone());
    }
    adj.insert(task_id, submit.depends_on.clone());
    adj
}

fn make_entry(
    task_id: String,
    submit: &TaskSubmit,
    provider: String,
    model: String,
    thinking: String,
    resolved_dir: &Path,
    priority: String,
) -> TaskEntry {
    let name = if submit.name.trim().is_empty() { task_id.clone() } else { submit.name.clone() };
    let preview: String = submit.prompt.chars().take(PROMPT_PREVIEW_LEN).collect();
    TaskEntry {
        task_id,
        chat_id: submit.chat_id,
        parent_agent: submit.parent_agent.clone(),
        name,
        prompt_preview: preview,
        provider,
        model,
        status: "running".to_string(),
        session_id: String::new(),
        created_at: chrono::Utc::now().timestamp() as f64,
        completed_at: 0.0,
        elapsed_seconds: 0.0,
        error: String::new(),
        result_preview: String::new(),
        question_count: 0,
        num_turns: 0,
        last_question: String::new(),
        original_prompt: submit.prompt.clone(),
        thinking,
        tasks_dir: resolved_dir.to_string_lossy().to_string(),
        thread_id: submit.thread_id,
        priority,
        depends_on: submit.depends_on.clone(),
    }
}

impl TaskRegistry {
    /// Creates or loads a TaskRegistry from a JSON file.
    pub fn new(registry_path: PathBuf, default_tasks_dir: PathBuf) -> Result<Self> {
        let registry = Self {
            registry_path,
            default_tasks_dir,
            entries: Mutex::new(HashMap::new()),
        };
        registry.load()?;
        registry.cleanup_orphans()?;
        Ok(registry)
    }

    fn load(&self) -> Result<()> {
        if !self.registry_path.exists() {
            return Ok(());
        }
        let content = fs::read_to_string(&self.registry_path)
            .context("Failed to read task registry file")?;
        if content.trim().is_empty() {
            return Ok(());
        }
        let data: RegistryData = serde_json::from_str(&content)
            .context("Failed to deserialize task registry")?;

        let mut entries = self.entries.lock().unwrap();
        for mut entry in data.tasks {
            if entry.status == "running" {
                entry.status = "failed".to_string();
                entry.error = "Bot restarted while task was running".to_string();
            }
            entries.insert(entry.task_id.clone(), entry);
        }
        Ok(())
    }

    pub(crate) fn persist(&self, entries: &HashMap<String, TaskEntry>) -> Result<()> {
        let data = RegistryData {
            tasks: entries.values().cloned().collect(),
        };
        let temp = self.registry_path.with_extension("tmp");
        let content = serde_json::to_string_pretty(&data)
            .context("Failed to serialize task registry")?;

        if let Some(parent) = self.registry_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&temp, content)?;
        fs::rename(&temp, &self.registry_path)?;
        Ok(())
    }

    /// Creates a new task entry in the registry and seeds its task folder.
    pub fn create(
        &self,
        submit: TaskSubmit,
        provider: String,
        model: String,
        thinking: String,
        tasks_dir_override: Option<PathBuf>,
        priority_override: Option<String>,
    ) -> Result<TaskEntry> {
        let mut entries = self.entries.lock().unwrap();

        let task_id = loop {
            let bytes: [u8; 4] = rand::random();
            let candidate = hex::encode(bytes);
            if !entries.contains_key(&candidate) {
                break candidate;
            }
        };

        let adj = get_adj(&entries, task_id.clone(), &submit);
        if check_cycle(&adj) {
            return Err(anyhow!("Cycle detected in task dependencies"));
        }

        let resolved_dir = tasks_dir_override.unwrap_or_else(|| self.default_tasks_dir.clone());
        let priority = normalise_priority(priority_override.as_deref().or(Some(&submit.priority)));

        let entry = make_entry(task_id.clone(), &submit, provider, model, thinking, &resolved_dir, priority);

        entries.insert(task_id.clone(), entry.clone());
        self.persist(&entries)?;

        seed_folder_helper(&resolved_dir, &task_id, &entry, &submit.prompt)?;

        Ok(entry)
    }

    /// Retrieve a task entry by its ID.
    pub fn get(&self, task_id: &str) -> Option<TaskEntry> {
        let entries = self.entries.lock().unwrap();
        entries.get(task_id).cloned()
    }

    /// Find a task by name within a chat.
    pub fn find_by_name(&self, chat_id: i64, name: &str) -> Option<TaskEntry> {
        let entries = self.entries.lock().unwrap();
        let lower = name.to_lowercase();
        for entry in entries.values() {
            if entry.chat_id == chat_id && entry.name.to_lowercase() == lower {
                return Some(entry.clone());
            }
        }
        None
    }

    /// List active running tasks.
    pub fn list_active(&self, chat_id: Option<i64>) -> Vec<TaskEntry> {
        let entries = self.entries.lock().unwrap();
        let mut active: Vec<TaskEntry> = entries
            .values()
            .filter(|e| e.status == "running" && chat_id.map_or(true, |c| e.chat_id == c))
            .cloned()
            .collect();
        active.sort_by(|a, b| a.created_at.partial_cmp(&b.created_at).unwrap());
        active
    }

    /// List all tasks.
    pub fn list_all(&self, chat_id: Option<i64>, parent_agent: Option<&str>) -> Vec<TaskEntry> {
        let entries = self.entries.lock().unwrap();
        let mut all: Vec<TaskEntry> = entries
            .values()
            .filter(|e| {
                chat_id.map_or(true, |c| e.chat_id == c)
                    && parent_agent.map_or(true, |pa| e.parent_agent == pa)
            })
            .cloned()
            .collect();
        all.sort_by(|a, b| b.created_at.partial_cmp(&a.created_at).unwrap());
        all
    }

    /// Update status and optional fields.
    pub fn update_status<F>(&self, task_id: &str, update_fn: F) -> Result<bool>
    where
        F: FnOnce(&mut TaskEntry),
    {
        let mut entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.get_mut(task_id) {
            update_fn(entry);
            self.persist(&entries)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Returns the task's workspace folder.
    pub fn task_folder(&self, task_id: &str) -> PathBuf {
        let entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.get(task_id) {
            self.task_folder_internal(entry)
        } else {
            self.default_tasks_dir.join(task_id)
        }
    }

    pub(crate) fn task_folder_internal(&self, entry: &TaskEntry) -> PathBuf {
        if !entry.tasks_dir.is_empty() {
            PathBuf::from(&entry.tasks_dir).join(&entry.task_id)
        } else {
            self.default_tasks_dir.join(&entry.task_id)
        }
    }

    /// Returns the path to TASKMEMORY.md for a given task.
    pub fn taskmemory_path(&self, task_id: &str) -> PathBuf {
        self.task_folder(task_id).join("TASKMEMORY.md")
    }
}

const TASK_RULES: &str = r#"# Task Agent Rules

You are a background task agent. You have NO direct user access.

## MANDATORY: Asking Questions

If you need ANY information to complete your task (missing details,
clarifications, user preferences), you MUST use this tool:

```bash
python3 tools/task_tools/ask_parent.py "your question here"
```

This forwards your question to the parent agent and returns immediately.
Do NOT write questions in your response — the user cannot see them.
After asking, finish your current work — you will be resumed with the answer.

## Other Tools (in `tools/task_tools/`)

- `python3 tools/task_tools/list_tasks.py` — List active tasks
- `python3 tools/task_tools/cancel_task.py TASK_ID` — Cancel a task
- `python3 tools/task_tools/delete_task.py TASK_ID` — Delete a finished task

## TASKMEMORY.md

Path: {taskmemory_path}

Update after completing your work:
- What you did and key decisions
- Results, file paths, or findings
"#;

fn seed_task_folder(folder: &Path, entry: &TaskEntry, prompt: &str) -> Result<()> {
    let taskmemory_path = folder.join("TASKMEMORY.md");
    if !taskmemory_path.exists() {
        let created = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let prompt_part: String = prompt.chars().take(500).collect();
        let initial_memory = format!(
            "# Task: {}\n\nCreated: {}\nProvider: {}/{}\n\n## Task Description\n\n{}\n\n## Progress\n\n_Update this section as you work._\n",
            entry.name, created, entry.provider, entry.model, prompt_part
        );
        fs::write(&taskmemory_path, initial_memory)?;
    }

    let rules_content = TASK_RULES.replace("{taskmemory_path}", &taskmemory_path.to_string_lossy());
    for name in &["CLAUDE.md", "AGENTS.md", "GEMINI.md"] {
        let rules_path = folder.join(name);
        fs::write(rules_path, &rules_content)?;
    }
    Ok(())
}
