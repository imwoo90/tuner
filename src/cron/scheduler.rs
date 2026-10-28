//! # Cron Engine Event Loop
//!
//! ## Overview
//! Continuously polls active cron jobs, evaluates timezone-adjusted upcoming runs, checks quiet hours,
//! and triggers background agent runs via [`AntigravityCli`].
//!
//! ## Search Tags
//! #cron-engine, #tick-loop, #quiet-hours, #task-spawner, #time-evaluator

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use chrono::{DateTime, NaiveTime, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use std::str::FromStr;

use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::cli::AgentProvider;
use crate::cron::manager::{CronJob, CronManager};

/// Represents `CronScheduler` (cron scheduler).
pub struct CronScheduler {
    /// Config.
    pub config: Arc<CliConfig>,
    /// Manager.
    pub manager: Arc<CronManager>,
    /// Bus.
    pub bus: Arc<crate::bus::bus::MessageBus>,
}

impl CronScheduler {
    /// Creates a new instance.
    pub fn new(
        config: Arc<CliConfig>,
        manager: Arc<CronManager>,
        bus: Arc<crate::bus::bus::MessageBus>,
    ) -> Self {
        Self { config, manager, bus }
    }

    pub(crate) fn effective_timezone<'a>(&'a self, job: &'a CronJob) -> &'a str {
        if !job.timezone.is_empty() {
            &job.timezone
        } else {
            self.config.user_timezone.as_deref().unwrap_or("UTC")
        }
    }

    pub(crate) fn calculate_next_run(&self, job: &CronJob) -> Result<DateTime<Utc>, String> {
        let next = calculate_job_next_run(&job.schedule, self.effective_timezone(job))?;
        Ok(next.with_timezone(&Utc))
    }

    /// Executes `start` (start).
    pub fn start(self: Arc<Self>) {
        let scheduler = self.clone();
        tokio::spawn(async move {
            let mut last_mtime: Option<SystemTime> = None;
            let mut next_run_times: HashMap<String, DateTime<Utc>> = HashMap::new();
            let mut loaded_jobs_configs: HashMap<String, (String, String)> = HashMap::new();
            let mut check_interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
            println!("🤖 [tuner] CronScheduler started");
            loop {
                check_interval.tick().await;
                let _ = scheduler.reload_if_changed(&mut last_mtime, &mut next_run_times, &mut loaded_jobs_configs).await;
                let _ = scheduler.run_due_jobs(&mut next_run_times).await;
            }
        });
    }

    async fn reload_if_changed(
        &self,
        last_mtime: &mut Option<SystemTime>,
        next_run_times: &mut HashMap<String, DateTime<Utc>>,
        loaded_jobs_configs: &mut HashMap<String, (String, String)>,
    ) -> Result<(), String> {
        let current_mtime = std::fs::metadata(self.manager.jobs_path()).ok().and_then(|m| m.modified().ok());
        if current_mtime != *last_mtime {
            *last_mtime = current_mtime;
            println!("🤖 [tuner] CronScheduler: reloading jobs");
            if let Ok(jobs) = self.manager.list_jobs().await {
                let mut updated_times = HashMap::new();
                let mut updated_configs = HashMap::new();
                for job in jobs.into_iter().filter(|j| j.enabled) {
                    let key = (job.schedule.clone(), job.timezone.clone());
                    let changed = loaded_jobs_configs.get(&job.id) != Some(&key);
                    if let Ok(next) = self.calculate_next_run(&job) {
                        let next_run = if changed {
                            next
                        } else {
                            next_run_times.get(&job.id).filter(|&&e| e > Utc::now()).copied().unwrap_or(next)
                        };
                        updated_times.insert(job.id.clone(), next_run);
                        updated_configs.insert(job.id, key);
                    }
                }
                *next_run_times = updated_times;
                *loaded_jobs_configs = updated_configs;
            }
        }
        Ok(())
    }

    async fn run_due_jobs(&self, next_run_times: &mut HashMap<String, DateTime<Utc>>) -> Result<(), String> {
        let now = Utc::now();
        let due_ids: Vec<String> = next_run_times.iter().filter(|(_, nr)| now >= **nr).map(|(id, _)| id.clone()).collect();
        for job_id in due_ids {
            if let Ok(Some(job)) = self.manager.get_job(&job_id).await {
                let s = Arc::new(Self::new(self.config.clone(), self.manager.clone(), self.bus.clone()));
                let j = job.clone();
                tokio::spawn(async move { let _ = s.execute_job(j).await; });
                if let Ok(next_run) = self.calculate_next_run(&job) {
                    next_run_times.insert(job_id, next_run);
                } else {
                    next_run_times.remove(&job_id);
                }
            }
        }
        Ok(())
    }

    /// Executes `check_quiet_hours_at` (check quiet hours at).
    pub fn check_quiet_hours_at(&self, job: &CronJob, now: DateTime<Utc>) -> bool {
        if let (Some(qs), Some(qe)) = (job.quiet_start, job.quiet_end)
            && let Ok(tz) = self.effective_timezone(job).parse::<Tz>()
        {
            let now_local = now.with_timezone(&tz);
            let start = NaiveTime::from_hms_opt(qs % 24, 0, 0).unwrap_or_default();
            let end = NaiveTime::from_hms_opt(qe % 24, 0, 0).unwrap_or_default();
            return crate::heartbeat::quiet::is_within_quiet_hours(&now_local, start, end);
        }
        false
    }

    fn get_enriched_prompt(&self, instruction: &str, folder: &str) -> String {
        format!(
            "{}\n\n\
             IMPORTANT:\n\
             - Read {}_MEMORY.md for crucial context.\n\
             - Update {}_MEMORY.md with timestamp and work summary when done.\n\
             - Telegram delivery is automatic. Return only user-facing result text.\n\
             - Do not include confirmation notes (e.g. \"Message sent\").",
            instruction, folder, folder
        )
    }

    async fn submit_cron_result(&self, title: &str, res: &str, stat: &str, cid: i64, tid: Option<i64>) {
        let chat_opt = if cid == 0 { None } else { Some(cid) };
        let mut env = crate::bus::adapters::from_cron_result(
            title, res, stat, chat_opt, tid, Some(&self.config.transport)
        );
        self.bus.submit(&mut env).await;
    }

    async fn run_cli_command(&self, job: &CronJob, workspace: std::path::PathBuf, cli: &AntigravityCli) -> Result<(), String> {
        let enriched = self.get_enriched_prompt(&job.agent_instruction, &job.task_folder);
        match cli.send(&enriched, None, false, workspace).await {
            Ok(resp) => {
                let status = if resp.is_error {
                    format!("error:exit_{}", resp.returncode.unwrap_or(1))
                } else {
                    "success".to_string()
                };
                self.manager.update_run_status(&job.id, &status).await?;
                if !job.silent_on_success || resp.is_error {
                    self.submit_cron_result(&job.title, &resp.result, &status, job.chat_id, job.topic_id).await;
                }
            }
            Err(e) => {
                let err_status = format!("error:{}", e);
                self.manager.update_run_status(&job.id, &err_status).await?;
                self.submit_cron_result(&job.title, "", &err_status, job.chat_id, job.topic_id).await;
            }
        }
        Ok(())
    }

    async fn execute_job(&self, job: CronJob) -> Result<(), String> {
        if self.check_quiet_hours_at(&job, Utc::now()) {
            println!("🤖 [tuner] Cron job {} skipped due to quiet hours", job.title);
            return Ok(());
        }
        self.run_job_core(job).await
    }

    /// Executes a cron job immediately on manual user request, bypassing quiet hours and silent mode.
    pub async fn execute_job_now(
        &self,
        mut job: CronJob,
        caller_chat_id: i64,
        caller_topic_id: Option<i64>,
    ) -> Result<(), String> {
        if job.chat_id == 0 {
            job.chat_id = caller_chat_id;
            job.topic_id = caller_topic_id;
        }
        job.silent_on_success = false;
        self.run_job_core(job).await
    }

    async fn run_job_core(&self, job: CronJob) -> Result<(), String> {
        println!("🤖 [tuner] Cron job executing: {}", job.title);
        let ctr = self.config.working_dir.join("cron_tasks");
        let workspace = ctr.join(&job.task_folder);
        if !crate::security::paths::is_path_safe(&workspace, &[ctr]) {
            let _ = self.manager.update_run_status(&job.id, "error:path_outside_allowed_roots").await;
            return Err("Cron task folder outside allowed roots".into());
        }
        if !workspace.is_dir() {
            let err_msg = format!("Cron task folder missing: {}", workspace.display());
            eprintln!("❌ [tuner] {}", err_msg);
            self.manager.update_run_status(&job.id, "error:folder_missing").await?;
            return Err(err_msg);
        }

        let mut jc = (*self.config).clone();
        if let Some(prof_dir) = self.manager.jobs_path().parent() {
            let prof_cfg = prof_dir.join("config").join("config.json");
            if prof_cfg.is_file() {
                let _ = jc.merge_profile_file(&prof_cfg);
            }
        }
        jc.working_dir = workspace.clone();
        jc.chat_id = job.chat_id;
        jc.topic_id = job.topic_id;
        jc.transport = job.transport.clone();
        if let Some(m) = &job.model { jc.model = Some(m.clone()); }
        if let Some(p) = &job.provider { jc.provider = p.clone(); }
        if !job.cli_parameters.is_empty() {
            jc.cli_parameters.insert("antigravity".to_string(), job.cli_parameters.clone());
        }
        if let Some(r) = &job.effort {
            jc.effort = Some(r.clone());
        }

        let job_cli = AntigravityCli::new(jc);
        self.run_cli_command(&job, workspace, &job_cli).await
    }
}

/// Executes `calculate_job_next_run` (calculate job next run).
pub fn calculate_job_next_run(schedule: &str, timezone: &str) -> Result<DateTime<Tz>, String> {
    let norm = if schedule.split_whitespace().count() == 5 {
        format!("0 {}", schedule)
    } else {
        schedule.to_string()
    };
    let sched = Schedule::from_str(&norm).map_err(|e| e.to_string())?;
    let tz_s = if timezone.is_empty() { "UTC" } else { timezone };
    let tz: Tz = tz_s.parse().map_err(|e| format!("Invalid timezone: {}", e))?;
    let next = sched.upcoming(tz).next().ok_or_else(|| "No upcoming run".to_string())?;
    Ok(next)
}
