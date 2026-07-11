use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use chrono::{DateTime, NaiveTime, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use std::str::FromStr;
use teloxide::prelude::*;

use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::cli::AgentProvider;
use crate::cron::manager::{CronJob, CronManager};

pub struct CronScheduler {
    pub config: Arc<CliConfig>,
    pub manager: Arc<CronManager>,
    pub cli: Arc<AntigravityCli>,
}

impl CronScheduler {
    pub fn new(
        config: Arc<CliConfig>,
        manager: Arc<CronManager>,
        cli: Arc<AntigravityCli>,
    ) -> Self {
        Self {
            config,
            manager,
            cli,
        }
    }

    fn normalize_cron_expression(expr: &str) -> String {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if parts.len() == 5 {
            format!("0 {}", expr)
        } else {
            expr.to_string()
        }
    }

    pub(crate) fn calculate_next_run(&self, job: &CronJob) -> Result<DateTime<Utc>, String> {
        let normalized = Self::normalize_cron_expression(&job.schedule);
        let sched = Schedule::from_str(&normalized).map_err(|e| e.to_string())?;

        let tz_str = if job.timezone.is_empty() {
            self.config.telegram_heartbeat_ack_token.as_ref()
                .map(|_| "UTC")
                .unwrap_or("UTC")
        } else {
            &job.timezone
        };
        let tz: Tz = tz_str.parse().map_err(|e| format!("Invalid timezone: {}", e))?;

        let next_local = sched.upcoming(tz).next()
            .ok_or_else(|| "No upcoming execution time found".to_string())?;

        Ok(next_local.with_timezone(&Utc))
    }

    pub fn start(self: Arc<Self>, bot: Bot) {
        let scheduler = self.clone();
        tokio::spawn(async move {
            let mut last_mtime: Option<SystemTime> = None;
            let mut next_run_times: HashMap<String, DateTime<Utc>> = HashMap::new();
            let mut check_interval = tokio::time::interval(tokio::time::Duration::from_secs(5));

            println!("🤖 [우덕터] CronScheduler started");

            loop {
                check_interval.tick().await;
                let _ = scheduler.tick(&mut last_mtime, &mut next_run_times, &bot).await;
            }
        });
    }

    async fn tick(
        &self,
        last_mtime: &mut Option<SystemTime>,
        next_run_times: &mut HashMap<String, DateTime<Utc>>,
        bot: &Bot,
    ) -> Result<(), String> {
        let _ = self.reload_if_changed(last_mtime, next_run_times).await;
        let _ = self.run_due_jobs(next_run_times, bot).await;
        Ok(())
    }

    async fn reload_if_changed(
        &self,
        last_mtime: &mut Option<SystemTime>,
        next_run_times: &mut HashMap<String, DateTime<Utc>>,
    ) -> Result<(), String> {
        let current_mtime = std::fs::metadata(self.manager.jobs_path())
            .ok()
            .and_then(|m| m.modified().ok());

        if current_mtime != *last_mtime {
            *last_mtime = current_mtime;
            println!("🤖 [우덕터] CronScheduler: reloading jobs");
            if let Ok(jobs) = self.manager.list_jobs().await {
                let mut updated_times = HashMap::new();
                for job in jobs {
                    if job.enabled {
                        if let Ok(next_run) = self.calculate_next_run(&job) {
                            if let Some(existing) = next_run_times.get(&job.id) {
                                if *existing > Utc::now() {
                                    updated_times.insert(job.id, *existing);
                                    continue;
                                }
                            }
                            updated_times.insert(job.id, next_run);
                        }
                    }
                }
                *next_run_times = updated_times;
            }
        }
        Ok(())
    }

    async fn run_due_jobs(
        &self,
        next_run_times: &mut HashMap<String, DateTime<Utc>>,
        bot: &Bot,
    ) -> Result<(), String> {
        let now = Utc::now();
        let mut jobs_to_run = Vec::new();

        for (job_id, next_run) in next_run_times.iter() {
            if now >= *next_run {
                jobs_to_run.push(job_id.clone());
            }
        }

        for job_id in jobs_to_run {
            if let Ok(Some(job)) = self.manager.get_job(&job_id).await {
                let bot_clone = bot.clone();
                let self_clone = Arc::new(Self::new(self.config.clone(), self.manager.clone(), self.cli.clone()));
                let job_for_task = job.clone();
                
                tokio::spawn(async move {
                    let _ = self_clone.execute_job(job_for_task, bot_clone).await;
                });

                if let Ok(next_run) = self.calculate_next_run(&job) {
                    next_run_times.insert(job_id, next_run);
                } else {
                    next_run_times.remove(&job_id);
                }
            }
        }
        Ok(())
    }

    async fn send_telegram(&self, bot: &Bot, chat_id: i64, topic_id: Option<i64>, text: &str) {
        let mut req = bot.send_message(ChatId(chat_id), text.to_string())
            .parse_mode(teloxide::types::ParseMode::Html);
        if let Some(tid) = topic_id {
            req = req.message_thread_id(tid as i32);
        }
        let _ = req.await;
    }

    fn check_quiet_hours(&self, job: &CronJob) -> bool {
        if let (Some(qs), Some(qe)) = (job.quiet_start, job.quiet_end) {
            let tz_str = if job.timezone.is_empty() { "UTC" } else { &job.timezone };
            if let Ok(tz) = tz_str.parse::<Tz>() {
                let now_local = Utc::now().with_timezone(&tz);
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
             - Read the {}_MEMORY.md file (it contains important information!)\n\
             - When finished, update {}_MEMORY.md with DATE + TIME and what you have done.\n\
             - The final answer is delivered to Telegram automatically by ductor.\n\
             - Return only the user-facing result text.\n\
             - Do not include transport/debug/tool confirmations (for example: \"Message sent successfully\").",
            instruction, folder, folder
        )
    }

    async fn execute_job(&self, job: CronJob, bot: Bot) -> Result<(), String> {
        let job_id = job.id.clone();
        let job_title = job.title.clone();
        let chat_id = job.chat_id;
        let topic_id = job.topic_id;

        if self.check_quiet_hours(&job) {
            println!("🤖 [우덕터] Cron job {} skipped due to quiet hours", job_title);
            return Ok(());
        }

        println!("🤖 [우덕터] Cron job executing: {}", job_title);

        let workspace = self.config.working_dir.join("cron_tasks").join(&job.task_folder);
        if !workspace.is_dir() {
            let err_msg = format!("Cron task folder missing: {}", workspace.display());
            eprintln!("❌ [우덕터] {}", err_msg);
            self.manager.update_run_status(&job_id, "error:folder_missing").await?;
            return Err(err_msg);
        }

        let enriched = self.get_enriched_prompt(&job.agent_instruction, &job.task_folder);
        let res = self.cli.send(&enriched, None, false, workspace).await;
        match res {
            Ok(resp) => {
                let status = if resp.is_error {
                    format!("error:exit_{}", resp.returncode.unwrap_or(1))
                } else {
                    "success".to_string()
                };

                self.manager.update_run_status(&job_id, &status).await?;

                if !job.silent_on_success || resp.is_error {
                    let html_text = crate::telegram::formatting::markdown_to_telegram_html(&resp.result);
                    self.send_telegram(&bot, chat_id, topic_id, &html_text).await;
                }
            }
            Err(e) => {
                self.manager.update_run_status(&job_id, &format!("error:{}", e)).await?;
                self.send_telegram(&bot, chat_id, topic_id, &format!("❌ Cron job '{}' failed: {}", job_title, e)).await;
            }
        }

        Ok(())
    }
}
