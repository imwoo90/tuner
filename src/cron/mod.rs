//! # Cron Scheduler & Job Automation Module (index.md)
//!
//! ## Overview
//! Provides a timezone-aware scheduler allowing automatic, periodic runs of AI agents in specified folders.
//!
//! ## Submodules
//! - [`manager`] (`manager.rs`): Handles listing, saving, enabling/disabling, and persisting scheduled jobs in JSON files.
//! - [`scheduler`] (`scheduler.rs`): Runs the ticks, monitors file modification times, checks quiet hours, and spawns tokio tasks.
//!
//! ## Search Tags
//! #cron-scheduler, #job-automation, #periodic-runs, #cron-manager, #timezone-evaluation

pub mod manager;
pub mod scheduler;

pub use manager::{CronJob, CronManager};
pub use scheduler::CronScheduler;

use chrono::DateTime;
use chrono_tz::Tz;
use cron::Schedule;
use std::path::Path;
use std::str::FromStr;

/// Computes the next timezone-aware execution timestamp for a 5- or 6-field cron expression.
pub fn calculate_job_next_run(schedule: &str, timezone: &str) -> Result<DateTime<Tz>, String> {
    let norm = if schedule.split_whitespace().count() == 5 {
        format!("0 {}", schedule)
    } else {
        schedule.to_string()
    };
    let sched = Schedule::from_str(&norm).map_err(|e| e.to_string())?;
    let tz_s = if timezone.is_empty() { "UTC" } else { timezone };
    let tz: Tz = tz_s.parse().map_err(|e| format!("Invalid timezone: {}", e))?;
    sched
        .upcoming(tz)
        .next()
        .ok_or_else(|| "No upcoming run".to_string())
}

pub(crate) fn build_job_cli_config(
    base: &crate::config::CliConfig,
    jobs_path: &Path,
    job: &CronJob,
    workspace: std::path::PathBuf,
) -> crate::config::CliConfig {
    let mut jc = base.clone();
    if let Some(prof_dir) = jobs_path.parent() {
        let prof_cfg = prof_dir.join("config").join("config.json");
        if prof_cfg.is_file() {
            let _ = jc.merge_profile_file(&prof_cfg);
        }
    }
    jc.working_dir = workspace;
    jc.chat_id = job.chat_id;
    jc.topic_id = job.topic_id;
    jc.transport = job.transport.clone();
    if let Some(p) = job.provider.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        jc.provider = crate::session::data::SessionData::canonical_provider(p)
            .unwrap_or("antigravity")
            .to_string();
    } else {
        jc.provider = crate::session::data::SessionData::effective_provider(&jc.provider, "antigravity")
            .to_string();
    }
    let eff_prov = jc.provider.clone();
    if !job.cli_parameters.is_empty() {
        jc.cli_parameters.insert(eff_prov.clone(), job.cli_parameters.clone());
    }
    apply_job_model_effort(&mut jc, &eff_prov, job);
    jc
}

fn apply_job_model_effort(jc: &mut crate::config::CliConfig, eff_prov: &str, job: &CronJob) {
    use crate::session::data::SessionData;
    let job_m = job.model.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let raw_m = job_m.or(jc.model.as_deref());
    let job_suf = job_m
        .filter(|m| SessionData::is_model_compatible(m, eff_prov))
        .and_then(|m| crate::cli::common::strip_effort_suffix(m).1)
        .map(str::to_string);
    let suf_eff = raw_m
        .filter(|m| SessionData::is_model_compatible(m, eff_prov))
        .and_then(|m| crate::cli::common::strip_effort_suffix(m).1)
        .map(str::to_string);
    let resolved_m = SessionData::default_model_for_provider(eff_prov, raw_m);
    jc.model = Some(resolved_m.clone());
    if let Some(r) = job.effort.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        jc.effort = Some(r.to_string());
    } else if let Some(se) = job_suf {
        jc.effort = Some(se);
    } else if jc.effort.is_none() {
        jc.effort = suf_eff;
    }
    if !crate::messenger::telegram::reply_directives::session_supports_effort(eff_prov, &resolved_m) {
        jc.effort = None;
    } else if let Some(ref eff) = jc.effort {
        jc.effort = crate::messenger::telegram::reply_directives::clamp_session_effort(
            eff_prov,
            &resolved_m,
            eff,
        );
    }
}
