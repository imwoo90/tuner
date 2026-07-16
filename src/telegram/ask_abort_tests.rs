#[cfg(test)]
mod tests {
    use crate::telegram::{handle_message, TopicNameCache, BotInfo};
    use crate::config::CliConfig;
    use crate::cli::antigravity::AntigravityCli;
    use crate::session::key::SessionKey;
    use crate::session::manager::SessionManager;
    use crate::cron::manager::CronManager;
    use std::sync::Arc;
    use teloxide::Bot;
    use teloxide::types::Message;

    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct EnvGuard {
        p: String,
        h: String,
    }

    impl EnvGuard {
        fn new(temp: &tempfile::TempDir) -> Self {
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

    fn mock_brain_dir(temp_dir: &tempfile::TempDir, sess_id: &str) {
        let p = temp_dir.path().join(format!(".gemini/antigravity-cli/brain/{}/.system_generated/logs", sess_id));
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("transcript_full.jsonl"), "").unwrap();
    }

    fn make_message_json(chat_id: i64, user_id: i64, text: &str) -> Message {
        let raw = format!(
            r#"{{"message_id":100,"date":1,"chat":{{"id":{},"type":"private"}},"from":{{"id":{},"is_bot":false,"first_name":"User"}},"text":"{}"}}"#,
            chat_id, user_id, text
        );
        serde_json::from_str(&raw).unwrap()
    }

    fn write_mock_agy(dir: &std::path::Path) {
        let agy_path = dir.join("agy");
        std::fs::write(&agy_path, "#!/bin/sh\necho '{\"result\":\"Dummy\",\"is_error\":false}'\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&agy_path).unwrap().permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&agy_path, perms);
        }
    }

    struct TestContext {
        mgr: Arc<SessionManager>,
        cfg: Arc<CliConfig>,
        cli: Arc<AntigravityCli>,
        bot: Bot,
        cron_mgr: Arc<CronManager>,
        topic_cache: Arc<TopicNameCache>,
        bot_info: Arc<BotInfo>,
    }

    fn init_test_context(temp_dir: &tempfile::TempDir) -> TestContext {
        let db_file = temp_dir.path().join("sessions.db");
        let mgr = Arc::new(SessionManager::new(db_file, 30, 4, false, "UTC".to_string(), None));
        let cfg = Arc::new(CliConfig {
            provider: "antigravity".to_string(),
            allowed_user_ids: vec![100],
            working_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        });
        let cli = Arc::new(AntigravityCli::new((*cfg).clone()));
        let bot = Bot::new("123:abc");
        let cron_mgr = Arc::new(CronManager::new(temp_dir.path().join("cron.json")));
        let topic_cache = Arc::new(TopicNameCache::new());
        let bot_info = Arc::new(BotInfo { username: Some("mock_bot".to_string()) });
        TestContext { mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info }
    }

    #[tokio::test]
    async fn test_telegram_active_ask_aborts_on_new_message() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let temp_dir = tempfile::tempdir().unwrap();
        let session_id = "test-session-abort-ask";
        mock_brain_dir(&temp_dir, session_id);
        write_mock_agy(temp_dir.path());
        let _env = EnvGuard::new(&temp_dir);
        let ctx = init_test_context(&temp_dir);
        let chat_id = 999;
        let key = SessionKey::telegram(chat_id, None);
        let (sess, _) = ctx.mgr.resolve_session(&key, &ctx.cfg.provider, "opus").await.unwrap();
        let mut updated = sess.clone();
        updated.set_session_id("antigravity", session_id);
        ctx.mgr.update_session(&updated, 0.05, 1).await.unwrap();
        let mut env = std::collections::HashMap::new();
        env.insert("TUNER_CHAT_ID".to_string(), chat_id.to_string());
        ctx.cli.sessions.ensure_session(session_id, temp_dir.path(), "sleep", &["10".to_string()], &env).await.unwrap();
        assert!(ctx.cli.sessions.is_active(session_id).await);
        ctx.cli.sessions.set_running(session_id, true).await;
        ctx.cli.sessions.set_ask_active(session_id, true).await;
        assert!(ctx.cli.sessions.is_ask_active(session_id).await);
        let msg = make_message_json(chat_id, 100, "Abort existing ask");
        let res = handle_message(ctx.bot, msg, ctx.cfg, ctx.mgr, ctx.cli.clone(), ctx.cron_mgr, ctx.topic_cache, ctx.bot_info).await;
        assert!(res.is_ok());
        assert!(!ctx.cli.sessions.is_active(session_id).await);
    }
}
