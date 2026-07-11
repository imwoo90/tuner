//! # Cleanup Observer Tests
//!
//! This module validates the deletion of expired files and directory pruning
//! in the file cleanup observer system.

#[cfg(test)]
mod tests {
    use crate::cleanup::observer::{delete_old_files, CleanupObserver, CleanupConfig};
    use std::fs;
    use std::time::{SystemTime, Duration};

    #[test]
    fn test_delete_old_files_removes_expired() {
        let temp = tempfile::tempdir().unwrap();
        let old_file = temp.path().join("old.txt");
        let recent_file = temp.path().join("recent.txt");

        // Write files
        fs::write(&old_file, "old contents").unwrap();
        fs::write(&recent_file, "recent contents").unwrap();

        // Backdate mtime of old_file by 40 days (40 * 86400 secs)
        let forty_days_ago = SystemTime::now() - Duration::from_secs(40 * 86400);
        filetime::set_file_mtime(&old_file, filetime::FileTime::from_system_time(forty_days_ago)).unwrap();

        // Keep recent_file current
        let now = SystemTime::now();
        filetime::set_file_mtime(&recent_file, filetime::FileTime::from_system_time(now)).unwrap();

        // Run cleanup with max_age_days = 30
        let deleted = delete_old_files(temp.path(), 30);
        
        assert_eq!(deleted, 1);
        assert!(!old_file.exists());
        assert!(recent_file.exists());
    }

    #[test]
    fn test_delete_old_files_recurses_into_subdirectories() {
        let temp = tempfile::tempdir().unwrap();
        let subdir = temp.path().join("2026-07-01");
        fs::create_dir_all(&subdir).unwrap();

        let old_file = subdir.join("old.txt");
        fs::write(&old_file, "old contents").unwrap();

        let forty_days_ago = SystemTime::now() - Duration::from_secs(40 * 86400);
        filetime::set_file_mtime(&old_file, filetime::FileTime::from_system_time(forty_days_ago)).unwrap();

        let deleted = delete_old_files(temp.path(), 30);
        
        assert_eq!(deleted, 1);
        assert!(!old_file.exists());
        // Empty subdirectory should be pruned
        assert!(!subdir.exists());
    }

    #[test]
    fn test_delete_old_files_keeps_subdir_with_recent_files() {
        let temp = tempfile::tempdir().unwrap();
        let subdir = temp.path().join("2026-07-11");
        fs::create_dir_all(&subdir).unwrap();

        let recent_file = subdir.join("recent.txt");
        fs::write(&recent_file, "recent contents").unwrap();

        let deleted = delete_old_files(temp.path(), 30);
        
        assert_eq!(deleted, 0);
        assert!(recent_file.exists());
        assert!(subdir.is_dir());
    }

    #[tokio::test]
    async fn test_cleanup_observer_execute() {
        let temp = tempfile::tempdir().unwrap();
        
        let tg_dir = temp.path().join("telegram_files");
        let out_dir = temp.path().join("output_to_user");
        
        fs::create_dir_all(&tg_dir).unwrap();
        fs::create_dir_all(&out_dir).unwrap();

        let old_tg = tg_dir.join("old.jpg");
        let old_out = out_dir.join("old.pdf");
        let recent_tg = tg_dir.join("new.jpg");

        fs::write(&old_tg, "img").unwrap();
        fs::write(&old_out, "pdf").unwrap();
        fs::write(&recent_tg, "img").unwrap();

        let forty_days_ago = SystemTime::now() - Duration::from_secs(40 * 86400);
        filetime::set_file_mtime(&old_tg, filetime::FileTime::from_system_time(forty_days_ago)).unwrap();
        filetime::set_file_mtime(&old_out, filetime::FileTime::from_system_time(forty_days_ago)).unwrap();

        let config = CleanupConfig {
            enabled: true,
            media_files_days: 30,
            output_to_user_days: 30,
            check_hour: 3,
        };

        let observer = CleanupObserver::new(config, tg_dir.clone(), out_dir.clone());
        let (del_tg, del_out) = observer.execute().await;

        assert_eq!(del_tg, 1);
        assert_eq!(del_out, 1);
        assert!(!old_tg.exists());
        assert!(!old_out.exists());
        assert!(recent_tg.exists());
    }
}
