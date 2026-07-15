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

        // Verify initial list is empty
        let jobs = manager.list_jobs().await.unwrap();
        assert!(jobs.is_empty());

        // Add job
        let job = CronJob::new(
            "job1".to_string(),
            "Title".to_string(),
            "0 9 * * *".to_string(),
            "folder".to_string(),
            "instruction".to_string(),
            100,
            None,
        );
        manager.add_job(job).await.unwrap();

        // Get job
        let retrieved = manager.get_job("job1").await.unwrap().unwrap();
        assert_eq!(retrieved.title, "Title");

        // Set disabled
        let changed = manager.set_enabled("job1", false).await.unwrap();
        assert!(changed);
        let retrieved_disabled = manager.get_job("job1").await.unwrap().unwrap();
        assert!(!retrieved_disabled.enabled);

        // Remove job
        let removed = manager.remove_job("job1").await.unwrap();
        assert!(removed);
        assert!(manager.list_jobs().await.unwrap().is_empty());
    }

    #[test]
    fn test_cron_scheduler_next_run() {
        let cfg = Arc::new(CliConfig::default());
        let cli = Arc::new(AntigravityCli::new((*cfg).clone()));
        let temp = NamedTempFile::new().unwrap();
        let manager = Arc::new(CronManager::new(temp.path().to_path_buf()));
        let bus = Arc::new(crate::bus::bus::MessageBus::new());
        let scheduler = CronScheduler::new(cfg, manager, cli, bus);



        let job = CronJob::new(
            "job1".to_string(),
            "Title".to_string(),
            "0 9 * * *".to_string(),
            "folder".to_string(),
            "instruction".to_string(),
            100,
            None,
        );

        let next_run = scheduler.calculate_next_run(&job);
        assert!(next_run.is_ok());
        let next_time = next_run.unwrap();
        assert!(next_time > chrono::Utc::now());
    }

    #[tokio::test]
    async fn test_cron_manager_duplicate_job_rejected() {
        let temp = NamedTempFile::new().unwrap();
        let manager = CronManager::new(temp.path().to_path_buf());

        let job = CronJob::new(
            "dup1".to_string(), "T".to_string(), "*/5 * * * *".to_string(),
            "dir".to_string(), "task".to_string(), 100, None,
        );
        manager.add_job(job.clone()).await.unwrap();
        let result = manager.add_job(job).await;
        assert!(result.is_err(), "Adding a duplicate job should return Err");
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

        for id in &["job-a", "job-b"] {
            let job = CronJob::new(
                id.to_string(), "T".to_string(), "*/5 * * * *".to_string(),
                "dir".to_string(), "task".to_string(), 100, None,
            );
            manager.add_job(job).await.unwrap();
        }

        manager.set_all_enabled(false).await.unwrap();
        for job in manager.list_jobs().await.unwrap() {
            assert!(!job.enabled, "All jobs should be disabled");
        }

        manager.set_all_enabled(true).await.unwrap();
        for job in manager.list_jobs().await.unwrap() {
            assert!(job.enabled, "All jobs should be enabled");
        }
    }

    fn setup_scheduler(quiet_start: Option<u32>, quiet_end: Option<u32>) -> CronScheduler {
        let temp = NamedTempFile::new().unwrap();
        let manager = Arc::new(CronManager::new(temp.path().to_path_buf()));
        let bus = Arc::new(crate::bus::bus::MessageBus::new());
        let mut cfg = CliConfig::default();
        cfg.telegram_heartbeat_quiet_start = quiet_start;
        cfg.telegram_heartbeat_quiet_end = quiet_end;
        cfg.telegram_heartbeat_enabled = quiet_start.is_some();
        let cfg = Arc::new(cfg);
        let cli = Arc::new(AntigravityCli::new((*cfg).clone()));
        CronScheduler::new(cfg, manager, cli, bus)
    }

    #[test]
    fn test_cron_quiet_hours() {
        use chrono::TimeZone;
        let scheduler = setup_scheduler(Some(21), Some(8));
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
        cfg.model = Some("opus".to_string());
        cfg.provider = "claude".to_string();

        let mut job = CronJob::new("test".into(), "T".into(), "*".into(), "task".into(), "do".into(), 0, None);
        job.model = Some("sonnet".to_string());
        
        let mut jc = cfg.clone();
        if let Some(m) = &job.model { jc.model = Some(m.clone()); }
        let cli = AntigravityCli::new(jc);
        let cmd = cli.build_command("Prompt", None, false);
        assert!(cmd.contains(&"--model".to_string()));
        let idx = cmd.iter().position(|s| s == "--model").unwrap();
        assert_eq!(cmd[idx + 1], "sonnet");
    }

    #[test]
    fn test_cron_override_cli() {
        let mut cfg = CliConfig::default();
        cfg.provider = "claude".to_string();

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
    fn test_cron_override_reasoning() {
        let mut cfg = CliConfig::default();
        cfg.provider = "claude".to_string();

        let mut job = CronJob::new("test".into(), "T".into(), "*".into(), "task".into(), "do".into(), 0, None);
        job.reasoning_effort = Some("high".to_string());
        
        let mut jc = cfg.clone();
        if let Some(r) = &job.reasoning_effort {
            jc.cli_parameters.entry("antigravity".to_string()).or_default()
                .extend(vec!["-c".to_string(), format!("model_reasoning_effort={}", r)]);
        }
        let cli = AntigravityCli::new(jc);
        let cmd = cli.build_command("Prompt", None, false);
        assert!(cmd.contains(&"-c".to_string()));
        let c_idx = cmd.iter().position(|s| s == "-c").unwrap();
        assert_eq!(cmd[c_idx + 1], "model_reasoning_effort=high");
    }

    #[test]
    fn test_cron_override_fallback() {
        let mut cfg = CliConfig::default();
        cfg.model = Some("opus".to_string());
        cfg.provider = "claude".to_string();

        let mut job = CronJob::new("test".into(), "T".into(), "*".into(), "task".into(), "do".into(), 0, None);
        
        let mut jc = cfg.clone();
        if let Some(m) = &job.model { jc.model = Some(m.clone()); }
        let cli = AntigravityCli::new(jc);
        let cmd = cli.build_command("Prompt", None, false);
        let idx = cmd.iter().position(|s| s == "--model").unwrap();
        assert_eq!(cmd[idx + 1], "opus");

        job.provider = Some("antigravity".to_string());
        let mut jc2 = cfg.clone();
        if let Some(p) = &job.provider { jc2.provider = p.clone(); }
        assert_eq!(jc2.provider, "antigravity");
    }
}
