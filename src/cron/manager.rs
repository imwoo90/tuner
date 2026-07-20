//! # Cron Task Registry and State Manager
//!
//! ## Overview
//! Handles loading, saving, registering, and tracking execution states of [`CronJob`] definitions.
//! Employs asynchronous mutex locks to prevent race conditions during JSON file persistence.
//!
//! ## Collaboration Graph
//! - Reads/writes local json files representing scheduled configurations.
//! - Consulted by [`super::scheduler::CronScheduler`] to reload jobs when changes are detected on disk.
//!
//! ## Key Structures
//! - [`CronJob`]: State representation containing schedule, provider, timezone, models, and execution logs.
//! - [`CronManager`]: Service coordinator handling persistence and access locks.
//!
//! ## Search Tags
//! #cron-registry, #job-persistence, #state-lock, #cron-jobs

use std::fs;
use std::path::PathBuf;
use chrono::Utc;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub schedule: String,
    pub task_folder: String,
    pub agent_instruction: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub timezone: String,
    #[serde(default)]
    pub created_at: String,
    pub last_run_at: Option<String>,
    pub last_run_status: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub cli_parameters: Vec<String>,
    pub quiet_start: Option<u32>,
    pub quiet_end: Option<u32>,
    pub dependency: Option<String>,
    #[serde(default)]
    pub chat_id: i64,
    pub topic_id: Option<i64>,
    #[serde(default = "default_tg")]
    pub transport: String,
    #[serde(default)]
    pub silent_on_success: bool,
}

fn default_true() -> bool {
    true
}

fn default_tg() -> String {
    "tg".to_string()
}

impl CronJob {
    pub fn new(
        id: String,
        title: String,
        schedule: String,
        task_folder: String,
        agent_instruction: String,
        chat_id: i64,
        topic_id: Option<i64>,
    ) -> Self {
        Self {
            id,
            title,
            description: String::new(),
            schedule,
            task_folder,
            agent_instruction,
            enabled: true,
            timezone: String::new(),
            created_at: Utc::now().to_rfc3339(),
            last_run_at: None,
            last_run_status: None,
            provider: None,
            model: None,
            reasoning_effort: None,
            cli_parameters: Vec::new(),
            quiet_start: None,
            quiet_end: None,
            dependency: None,
            chat_id,
            topic_id,
            transport: "tg".to_string(),
            silent_on_success: false,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct CronJobsFile {
    pub jobs: Vec<CronJob>,
}

pub struct CronManager {
    jobs_path: PathBuf,
    lock: tokio::sync::Mutex<()>,
}

impl CronManager {
    pub fn new(jobs_path: PathBuf) -> Self {
        Self {
            jobs_path,
            lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn jobs_path(&self) -> &PathBuf {
        &self.jobs_path
    }

    pub fn load(&self) -> Result<Vec<CronJob>, String> {
        if !self.jobs_path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&self.jobs_path).map_err(|e| e.to_string())?;
        if content.trim().is_empty() {
            return Ok(Vec::new());
        }
        let file_data: CronJobsFile = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        Ok(file_data.jobs)
    }

    pub fn save(&self, jobs: &[CronJob]) -> Result<(), String> {
        let temp_path = self.jobs_path.with_extension("tmp");
        let file_data = CronJobsFile { jobs: jobs.to_vec() };
        let content = serde_json::to_string_pretty(&file_data).map_err(|e| e.to_string())?;
        if let Some(parent) = self.jobs_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&temp_path, content).map_err(|e| e.to_string())?;
        fs::rename(&temp_path, &self.jobs_path).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn add_job(&self, job: CronJob) -> Result<(), String> {
        let _guard = self.lock.lock().await;
        let mut jobs = self.load()?;
        if jobs.iter().any(|j| j.id == job.id) {
            return Err(format!("Job '{}' already exists", job.id));
        }
        jobs.push(job);
        self.save(&jobs)?;
        Ok(())
    }

    pub async fn remove_job(&self, job_id: &str) -> Result<bool, String> {
        let _guard = self.lock.lock().await;
        let mut jobs = self.load()?;
        let before_len = jobs.len();
        jobs.retain(|j| j.id != job_id);
        let removed = jobs.len() < before_len;
        if removed {
            self.save(&jobs)?;
        }
        Ok(removed)
    }

    pub async fn list_jobs(&self) -> Result<Vec<CronJob>, String> {
        let _guard = self.lock.lock().await;
        self.load()
    }

    pub async fn get_job(&self, job_id: &str) -> Result<Option<CronJob>, String> {
        let _guard = self.lock.lock().await;
        let jobs = self.load()?;
        Ok(jobs.into_iter().find(|j| j.id == job_id))
    }

    pub async fn set_enabled(&self, job_id: &str, enabled: bool) -> Result<bool, String> {
        let _guard = self.lock.lock().await;
        let mut jobs = self.load()?;
        let mut changed = false;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == job_id) {
            if job.enabled != enabled {
                job.enabled = enabled;
                changed = true;
            }
        }
        if changed {
            self.save(&jobs)?;
        }
        Ok(changed)
    }

    pub async fn set_all_enabled(&self, enabled: bool) -> Result<usize, String> {
        let _guard = self.lock.lock().await;
        let mut jobs = self.load()?;
        let mut changed_count = 0;
        for job in jobs.iter_mut() {
            if job.enabled != enabled {
                job.enabled = enabled;
                changed_count += 1;
            }
        }
        if changed_count > 0 {
            self.save(&jobs)?;
        }
        Ok(changed_count)
    }

    pub async fn update_run_status(&self, job_id: &str, status: &str) -> Result<(), String> {
        let _guard = self.lock.lock().await;
        let mut jobs = self.load()?;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == job_id) {
            job.last_run_at = Some(Utc::now().to_rfc3339());
            job.last_run_status = Some(status.to_string());
            self.save(&jobs)?;
        }
        Ok(())
    }
}
