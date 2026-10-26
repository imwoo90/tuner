#![allow(clippy::field_reassign_with_default)]
#[cfg(test)]
mod tests {
    use crate::cron::manager::{CronJob, CronManager};
    use crate::cron::scheduler::CronScheduler;
    use crate::config::CliConfig;
    use crate::cli::antigravity::AntigravityCli;
    use std::sync::Arc;
    use tempfile::NamedTempFile;

    #[test]
    fn test_cron_job_serialization() {
        let job = CronJob::new(
            "job1".to_string(),
            "Title".to_string(),
            "*/5 * * * *".to_string(),
            "task_dir".to_string(),
            "Do task".to_string(),
            12345,
            Some(678),
        );
        let serialized = serde_json::to_string(&job).unwrap();
        let deserialized: CronJob = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.id, "job1");
        assert_eq!(deserialized.schedule, "*/5 * * * *");
        assert!(deserialized.enabled);
    }

    #[tokio::test]
    async fn test_cron_manager_crud() {
        let temp = NamedTempFile::new().unwrap();
        let manager = CronManager::new(temp.path().to_path_buf());

        let jobs = manager.list_jobs().await.unwrap();
        assert!(jobs.is_empty());

        let job = CronJob::new(
            "job1".to_string(),
            "Title".to_string(),
            "0 9 * * *".to_string(),
            "folder".to_string(),
            "instruction".to_string(),
            100,
            None,
        );
        manager.save(&[job]).unwrap();

        let retrieved = manager.get_job("job1").await.unwrap().unwrap();
        assert_eq!(retrieved.title, "Title");

        let changed = manager.set_enabled("job1", false).await.unwrap();
        assert!(changed);
        let retrieved_disabled = manager.get_job("job1").await.unwrap().unwrap();
        assert!(!retrieved_disabled.enabled);

        manager.save(&[]).unwrap();
        assert!(manager.list_jobs().await.unwrap().is_empty());
    }

    #[test]
    fn test_cron_scheduler_next_run_and_user_timezone_fallback() {
        let mut cfg = CliConfig::default();
        cfg.user_timezone = Some("Asia/Seoul".to_string());
        let cfg = Arc::new(cfg);
        let temp = NamedTempFile::new().unwrap();
        let manager = Arc::new(CronManager::new(temp.path().to_path_buf()));
        let bus = Arc::new(crate::bus::bus::MessageBus::new());
        let scheduler = CronScheduler::new(cfg, manager, bus);

        let job = CronJob::new(
            "job1".to_string(),
            "Title".to_string(),
            "0 9 * * *".to_string(),
            "folder".to_string(),
            "instruction".to_string(),
            100,
            None,
        );
        assert_eq!(scheduler.effective_timezone(&job), "Asia/Seoul");

        let next_run = scheduler.calculate_next_run(&job);
        assert!(next_run.is_ok());
        let next_time = next_run.unwrap();
        assert!(next_time > chrono::Utc::now());
    }

    #[tokio::test]
    async fn test_cron_manager_get_nonexistent_returns_none() {
        let temp = NamedTempFile::new().unwrap();
        let manager = CronManager::new(temp.path().to_path_buf());
        let result = manager.get_job("nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_cron_manager_set_all_enabled() {
        let temp = NamedTempFile::new().unwrap();
        let manager = CronManager::new(temp.path().to_path_buf());

        let initial_jobs: Vec<CronJob> = ["job-a", "job-b"]
            .iter()
            .map(|id| {
                CronJob::new(
                    id.to_string(), "T".to_string(), "*/5 * * * *".to_string(),
                    "dir".to_string(), "task".to_string(), 100, None,
                )
            })
            .collect();
        manager.save(&initial_jobs).unwrap();

        manager.set_all_enabled(false).await.unwrap();
        for job in manager.list_jobs().await.unwrap() {
            assert!(!job.enabled, "All jobs should be disabled");
        }

        manager.set_all_enabled(true).await.unwrap();
        for job in manager.list_jobs().await.unwrap() {
            assert!(job.enabled, "All jobs should be enabled");
        }
    }

    fn setup_scheduler() -> CronScheduler {
        let temp = NamedTempFile::new().unwrap();
        let manager = Arc::new(CronManager::new(temp.path().to_path_buf()));
        let bus = Arc::new(crate::bus::bus::MessageBus::new());
        let cfg = Arc::new(CliConfig::default());
        CronScheduler::new(cfg, manager, bus)
    }

    #[test]
    fn test_cron_quiet_hours() {
        use chrono::TimeZone;
        let scheduler = setup_scheduler();
        let mut job = CronJob::new("test".into(), "T".into(), "*".into(), "task".into(), "do".into(), 0, None);

        let now = chrono::Utc.with_ymd_and_hms(2025, 6, 15, 23, 30, 0).unwrap();
        assert!(!scheduler.check_quiet_hours_at(&job, now));

        let now_active = chrono::Utc.with_ymd_and_hms(2025, 6, 15, 14, 0, 0).unwrap();
        assert!(!scheduler.check_quiet_hours_at(&job, now_active));

        job.quiet_start = Some(10);
        job.quiet_end = Some(16);
        let now_quiet = chrono::Utc.with_ymd_and_hms(2025, 6, 15, 14, 0, 0).unwrap();
        assert!(scheduler.check_quiet_hours_at(&job, now_quiet));

        job.quiet_start = Some(21);
        job.quiet_end = Some(8);
        let now_boundary_start = chrono::Utc.with_ymd_and_hms(2025, 6, 15, 21, 0, 0).unwrap();
        assert!(scheduler.check_quiet_hours_at(&job, now_boundary_start));

        let now_boundary_end = chrono::Utc.with_ymd_and_hms(2025, 6, 15, 8, 0, 0).unwrap();
        assert!(!scheduler.check_quiet_hours_at(&job, now_boundary_end));

        job.quiet_start = Some(0);
        job.quiet_end = Some(0);
        assert!(!scheduler.check_quiet_hours_at(&job, now_quiet));
    }

    #[test]
    fn test_cron_override_model() {
        let mut cfg = CliConfig::default();
        cfg.model = Some("gemini-3.8-flash".to_string());
        cfg.provider = "antigravity".to_string();

        let mut job = CronJob::new("test".into(), "T".into(), "*".into(), "task".into(), "do".into(), 0, None);
        job.model = Some("gemini-4-argon".to_string());

        let mut jc = cfg.clone();
        if let Some(m) = &job.model { jc.model = Some(m.clone()); }
        let cli = AntigravityCli::new(jc);
        let cmd = cli.build_command("Prompt", None, false);
        assert!(cmd.contains(&"--model".to_string()));
        let idx = cmd.iter().position(|s| s == "--model").unwrap();
        assert_eq!(cmd[idx + 1], "gemini-4-argon");
    }

    #[test]
    fn test_cron_override_cli() {
        let mut cfg = CliConfig::default();
        cfg.provider = "antigravity".to_string();

        let mut job = CronJob::new("test".into(), "T".into(), "*".into(), "task".into(), "do".into(), 0, None);
        job.cli_parameters = vec!["--chrome".to_string()];

        let mut jc = cfg.clone();
        if !job.cli_parameters.is_empty() {
            jc.cli_parameters.insert("antigravity".to_string(), job.cli_parameters.clone());
        }
        let cli = AntigravityCli::new(jc);
        let cmd = cli.build_command("Prompt", None, false);
        assert!(cmd.contains(&"--chrome".to_string()));
    }

    #[test]
    fn test_cron_override_effort() {
        let mut cfg = CliConfig::default();
        cfg.provider = "antigravity".to_string();
        cfg.model = Some("gemini-3.8-flash".to_string());

        let mut job = CronJob::new("test".into(), "T".into(), "*".into(), "task".into(), "do".into(), 0, None);
        job.effort = Some("low".to_string());

        let mut jc = cfg.clone();
        if let Some(r) = &job.effort {
            jc.effort = Some(r.clone());
        }
        let cli = AntigravityCli::new(jc);
        let cmd = cli.build_command("Prompt", None, false);
        assert!(cmd.contains(&"--effort".to_string()));
        let e_idx = cmd.iter().position(|s| s == "--effort").unwrap();
        assert_eq!(cmd[e_idx + 1], "low");
    }

    #[test]
    fn test_cron_override_fallback() {
        let mut cfg = CliConfig::default();
        cfg.model = Some("gemini-4-argon".to_string());
        cfg.provider = "antigravity".to_string();

        let job = CronJob::new("test".into(), "T".into(), "*".into(), "task".into(), "do".into(), 0, None);

        let mut jc = cfg.clone();
        if let Some(m) = &job.model { jc.model = Some(m.clone()); }
        let cli = AntigravityCli::new(jc);
        let cmd = cli.build_command("Prompt", None, false);
        let idx = cmd.iter().position(|s| s == "--model").unwrap();
        assert_eq!(cmd[idx + 1], "gemini-4-argon");
    }
}
