#[cfg(test)]
pub mod helpers {
    use crate::telegram::{TopicNameCache as TNC, BotInfo as BI};
    use crate::config::CliConfig as CC;
    use crate::cli::antigravity::AntigravityCli as AC;
    use crate::session::manager::SessionManager as SM;
    use crate::cron::manager::CronManager as CM;
    use std::sync::Arc;
    use teloxide::Bot;
    use teloxide::types::Message;

    pub struct EnvGuard {
        p: String,
        h: String,
    }

    impl EnvGuard {
        pub fn new(temp: &tempfile::TempDir) -> Self {
            let p = std::env::var("PATH").unwrap_or_default();
            let h = std::env::var("HOME").unwrap_or_default();
            let path_env = format!("{}:{}", temp.path().to_string_lossy(), p);
            unsafe {
                std::env::set_var("PATH", &path_env);
                std::env::set_var("HOME", temp.path().to_string_lossy().to_string());
            }
            Self { p, h }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                std::env::set_var("PATH", &self.p);
                std::env::set_var("HOME", &self.h);
            }
        }
    }

    pub type E2ETestEnv = (
        Arc<SM>,
        Arc<CC>,
        Arc<AC>,
        Bot,
        Arc<CM>,
        Arc<TNC>,
        Arc<BI>,
        Arc<crate::telegram::media_group::MediaGroupManager>,
        EnvGuard,
    );

    pub fn mock_brain_dir(temp_dir: &tempfile::TempDir, sess_id: &str) {
        let p = temp_dir.path().join(format!(".gemini/antigravity-cli/brain/{}/.system_generated/logs", sess_id));
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("transcript_full.jsonl"), "").unwrap();
    }

    pub fn setup_e2e_env(temp_dir: &tempfile::TempDir, mock_script_code: &str) -> E2ETestEnv {
        let agy_path = temp_dir.path().join("agy");
        std::fs::write(&agy_path, mock_script_code).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&agy_path).unwrap().permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&agy_path, perms);
        }
        let guard = EnvGuard::new(temp_dir);
        let db_file = temp_dir.path().join("sessions.db");
        let mgr = Arc::new(SM::new(db_file, 30, 4, false, "UTC".to_string(), None));
        let cfg = Arc::new(CC {
            provider: "antigravity".to_string(),
            allowed_user_ids: vec![100],
            allowed_group_ids: vec![123],
            working_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        });
        let cli = Arc::new(AC::new((*cfg).clone()));
        let bot = Bot::new("123:abc");
        let cron_mgr = Arc::new(CM::new(temp_dir.path().join("cron.json")));
        let topic_cache = Arc::new(TNC::new());
        let bot_info = Arc::new(BI { username: Some("mock_bot".to_string()) });
        let mgm = Arc::new(crate::telegram::media_group::MediaGroupManager::new());
        (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm, guard)
    }

    pub fn make_msg(json: &str) -> Message {
        serde_json::from_str(json).unwrap()
    }

    pub async fn wait_for_prompt(temp: &tempfile::TempDir, pattern: &str) -> String {
        let mut prompt_log = String::new();
        for _ in 0..50 {
            if let Ok(c) = std::fs::read_to_string(temp.path().join("received_prompts.txt")) {
                prompt_log = c;
                if prompt_log.contains(pattern) { break; }
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
        prompt_log
    }
}
