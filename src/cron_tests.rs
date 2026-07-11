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
        let scheduler = CronScheduler::new(cfg, manager, cli);

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
}
