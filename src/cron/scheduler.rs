//! # Cron Engine Event Loop
//!
//! ## Overview
//! Continuously polls active cron jobs, evaluates timezone-adjusted upcoming runs, checks quiet hours,
//! and triggers background agent runs via [`AntigravityCli`].
//!
//! ## Collaboration Graph
//! - Ticked by [`CronScheduler::start`] event loop spawning background monitoring threads.
//! - Queries [`super::manager::CronManager`] to retrieve active [`CronJob`] structures.
//! - Utilizes [`AntigravityCli`] to run prompt instructions.
//! - Dispatches results as envelopes to the central [`MessageBus`](crate::bus::bus::MessageBus).
//!
//! ## Key Types
//! - [`CronScheduler`]: Controller holding configuration, client bindings, message bus adapters, and quiet-hour status.
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

pub struct CronScheduler {
    pub config: Arc<CliConfig>,
    pub manager: Arc<CronManager>,
    pub cli: Arc<AntigravityCli>,
    pub bus: Arc<crate::bus::bus::MessageBus>,
}

impl CronScheduler {
    pub fn new(
        config: Arc<CliConfig>,
        manager: Arc<CronManager>,
        cli: Arc<AntigravityCli>,
        bus: Arc<crate::bus::bus::MessageBus>,
    ) -> Self {
        Self { config, manager, cli, bus }
    }

    fn normalize_cron_expression(expr: &str) -> String {
        if expr.split_whitespace().count() == 5 { format!("0 {}", expr) } else { expr.to_string() }
    }

    pub(crate) fn calculate_next_run(&self, job: &CronJob) -> Result<DateTime<Utc>, String> {
        let norm = Self::normalize_cron_expression(&job.schedule);
        let sched = Schedule::from_str(&norm).map_err(|e| e.to_string())?;
        let tz_s = if job.timezone.is_empty() { "UTC" } else { &job.timezone };
        let tz: Tz = tz_s.parse().map_err(|e| format!("Invalid timezone: {}", e))?;
        let next = sched.upcoming(tz).next().ok_or_else(|| "No upcoming run".to_string())?;
        Ok(next.with_timezone(&Utc))
    }

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
                let _ = scheduler.tick(&mut last_mtime, &mut next_run_times, &mut loaded_jobs_configs).await;
            }
        });
    }

    async fn tick(
        &self,
        last_mtime: &mut Option<SystemTime>,
        next_run_times: &mut HashMap<String, DateTime<Utc>>,
        loaded_jobs_configs: &mut HashMap<String, (String, String)>,
    ) -> Result<(), String> {
        let _ = self.reload_if_changed(last_mtime, next_run_times, loaded_jobs_configs).await;
        let _ = self.run_due_jobs(next_run_times).await;
        Ok(())
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
                for job in jobs {
                    if job.enabled {
                        let config_key = (job.schedule.clone(), job.timezone.clone());
                        let has_changed = match loaded_jobs_configs.get(&job.id) {
                            Some(old_cfg) => old_cfg != &config_key,
                            None => true,
                        };

                        if let Ok(next) = self.calculate_next_run(&job) {
                            let next_run = if has_changed {
                                next
                            } else {
                                next_run_times.get(&job.id).filter(|&&e| e > Utc::now()).copied().unwrap_or(next)
                            };
                            updated_times.insert(job.id.clone(), next_run);
                            updated_configs.insert(job.id, config_key);
                        }
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
        let mut jobs_to_run = Vec::new();
        for (job_id, next_run) in next_run_times.iter() {
            if now >= *next_run { jobs_to_run.push(job_id.clone()); }
        }
        for job_id in jobs_to_run {
            if let Ok(Some(job)) = self.manager.get_job(&job_id).await {
                let self_clone = Arc::new(Self::new(self.config.clone(), self.manager.clone(), self.cli.clone(), self.bus.clone()));
                let job_for_task = job.clone();
                tokio::spawn(async move { let _ = self_clone.execute_job(job_for_task).await; });
                if let Ok(next_run) = self.calculate_next_run(&job) {
                    next_run_times.insert(job_id, next_run);
                } else {
                    next_run_times.remove(&job_id);
                }
            }
        }
        Ok(())
    }

    fn check_quiet_hours(&self, job: &CronJob) -> bool {
        self.check_quiet_hours_at(job, Utc::now())
    }

    pub fn check_quiet_hours_at(&self, job: &CronJob, now: DateTime<Utc>) -> bool {
        if let (Some(qs), Some(qe)) = (job.quiet_start, job.quiet_end) {
            let tz_str = if job.timezone.is_empty() { "UTC" } else { &job.timezone };
            if let Ok(tz) = tz_str.parse::<Tz>() {
                let now_local = now.with_timezone(&tz);
                let start = NaiveTime::from_hms_opt(qs, 0, 0).unwrap();
                let end = NaiveTime::from_hms_opt(qe, 0, 0).unwrap();
                return crate::heartbeat::quiet::is_within_quiet_hours(&now_local, start, end);
            }
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
        let mut env = crate::bus::adapters::from_cron_result(
            title, res, stat, Some(cid), tid, Some(&self.config.transport)
        );
        self.bus.submit(&mut env).await;
    }

    async fn run_cli_command(&self, job: &CronJob, workspace: std::path::PathBuf, cli: &AntigravityCli) -> Result<(), String> {
        let job_id = job.id.clone();
        let job_title = job.title.clone();
        let chat_id = job.chat_id;
        let topic_id = job.topic_id;
        let enriched = self.get_enriched_prompt(&job.agent_instruction, &job.task_folder);
        let res = cli.send(&enriched, None, false, workspace).await;
        match res {
            Ok(resp) => {
                let status = if resp.is_error {
                    format!("error:exit_{}", resp.returncode.unwrap_or(1))
                } else {
                    "success".to_string()
                };
                self.manager.update_run_status(&job_id, &status).await?;
                if !job.silent_on_success || resp.is_error {
                    self.submit_cron_result(&job_title, &resp.result, &status, chat_id, topic_id).await;
                }
            }
            Err(e) => {
                let err_status = format!("error:{}", e);
                self.manager.update_run_status(&job_id, &err_status).await?;
                self.submit_cron_result(&job_title, "", &err_status, chat_id, topic_id).await;
            }
        }
        Ok(())
    }

    async fn execute_job(&self, job: CronJob) -> Result<(), String> {
        if self.check_quiet_hours(&job) {
            println!("🤖 [tuner] Cron job {} skipped due to quiet hours", job.title);
            return Ok(());
        }
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
        jc.chat_id = job.chat_id;
        jc.topic_id = job.topic_id;
        jc.transport = job.transport.clone();
        if let Some(m) = &job.model { jc.model = Some(m.clone()); }
        if let Some(p) = &job.provider { jc.provider = p.clone(); }
        if !job.cli_parameters.is_empty() {
            jc.cli_parameters.insert("antigravity".to_string(), job.cli_parameters.clone());
        }
        if let Some(r) = &job.reasoning_effort {
            jc.cli_parameters.entry("antigravity".to_string()).or_default()
                .extend(vec!["-c".to_string(), format!("model_reasoning_effort={}", r)]);
        }

        let job_cli = AntigravityCli::new(jc);
        self.run_cli_command(&job, workspace, &job_cli).await
    }
}
