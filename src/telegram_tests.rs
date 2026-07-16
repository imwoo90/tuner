//! # Telegram Integration Tests
//!
//! This module contains unit and integration tests for the Telegram bot interface,
//! validating message reply prompt building and SessionManager integration.

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

    type E2ETestEnv = (
        Arc<SessionManager>,
        Arc<CliConfig>,
        Arc<AntigravityCli>,
        Bot,
        Arc<CronManager>,
        Arc<TopicNameCache>,
        Arc<BotInfo>,
        EnvGuard
    );

    fn mock_brain_dir(temp_dir: &tempfile::TempDir, sess_id: &str) {
        let p = temp_dir.path().join(format!(".gemini/antigravity-cli/brain/{}/.system_generated/logs", sess_id));
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("transcript_full.jsonl"), "").unwrap();
    }

    fn setup_e2e_env(temp_dir: &tempfile::TempDir, mock_script_code: &str) -> E2ETestEnv {
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
        (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, guard)
    }

    fn make_message_json(chat_id: i64, user_id: i64, text: &str) -> Message {
        let raw = format!(
            r#"{{"message_id":100,"date":1,"chat":{{"id":{},"type":"private"}},"from":{{"id":{},"is_bot":false,"first_name":"User"}},"text":"{}"}}"#,
            chat_id, user_id, text
        );
        serde_json::from_str(&raw).unwrap()
    }

    #[tokio::test]
    async fn test_telegram_session_manager_integration() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mgr = Arc::new(SessionManager::new(temp.path().to_path_buf(), 30, 4, false, "UTC".to_string(), None));
        let cfg = Arc::new(CliConfig { provider: "antigravity".to_string(), allowed_user_ids: vec![100], ..Default::default() });
        let (sess, is_new) = mgr.resolve_session(&SessionKey::telegram(123, None), &cfg.provider, "opus").await.unwrap();
        assert!(is_new && sess.chat_id == 123);

        let msg = serde_json::from_str(r#"{"message_id":1,"date":1,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"text":"hello"}"#).unwrap();
        let tc = tempfile::NamedTempFile::new().unwrap();
        let cm = Arc::new(CronManager::new(tc.path().to_path_buf()));
        let cli = Arc::new(AntigravityCli::new((*cfg).clone()));
        assert!(handle_message(Bot::new("123:abc"), msg, cfg, mgr, cli, cm, Arc::new(TopicNameCache::new()), Arc::new(BotInfo { username: Some("my_bot".to_string()) })).await.is_ok());
    }

    #[tokio::test]
    async fn test_telegram_migrate_chat_id() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mgr = Arc::new(SessionManager::new(temp.path().to_path_buf(), 30, 4, false, "UTC".to_string(), None));
        let cfg = Arc::new(CliConfig { provider: "antigravity".to_string(), allowed_user_ids: vec![100], allowed_group_ids: vec![123, 456], ..Default::default() });

        let old_key = SessionKey::telegram(123, None);
        let (sess, _) = mgr.resolve_session(&old_key, &cfg.provider, "opus").await.unwrap();
        let mut updated = sess.clone();
        updated.set_session_id("antigravity", "migrated-session-123");
        mgr.update_session(&updated, 0.05, 100).await.unwrap();

        let cli = Arc::new(AntigravityCli::new((*cfg).clone()));
        let bot = Bot::new("123:abc");
        let msg = serde_json::from_str(r#"{"message_id":2,"date":1,"chat":{"id":123,"type":"group","title":"G"},"from":{"id":100,"is_bot":false,"first_name":"I"},"migrate_to_chat_id":456}"#).unwrap();
        let temp_cron = tempfile::NamedTempFile::new().unwrap();
        let cron_mgr = Arc::new(CronManager::new(temp_cron.path().to_path_buf()));
        let topic_cache = Arc::new(TopicNameCache::new());
        let bot_info = Arc::new(BotInfo { username: Some("my_bot".to_string()) });

        handle_message(bot, msg, cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info).await.unwrap();
        let new_sess = mgr.get_active(&SessionKey::telegram(456, None)).await.unwrap().unwrap();
        assert_eq!(new_sess.chat_id, 456);
        assert_eq!(new_sess.get_session_id("antigravity"), "migrated-session-123");
        assert!(mgr.get_active(&old_key).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_normal_message_flow_e2e() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let temp_dir = tempfile::tempdir().unwrap();
        mock_brain_dir(&temp_dir, "mock-session-123");
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, _env) = setup_e2e_env(&temp_dir, "#!/bin/sh\necho '{\"result\":\"Hello from mock agent!\",\"is_error\":false}'\n");
        let chat_id = 999;
        let key = SessionKey::telegram(chat_id, None);

        let msg = make_message_json(chat_id, 100, "Hello bot");
        let res1 = handle_message(bot.clone(), msg, cfg.clone(), mgr.clone(), cli.clone(), cron_mgr.clone(), topic_cache.clone(), bot_info.clone()).await;
        assert!(res1.is_ok(), "First handle_message failed: {:?}", res1);
        let active1 = mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(active1.provider_sessions.get("antigravity").map(|p| p.message_count), Some(1));

        std::fs::write(temp_dir.path().join("agy"), "#!/bin/sh\nfor arg in \"$@\"; do\n  if [ \"$arg\" = \"--conversation\" ]; then\n    echo '{\"result\":\"Resumed successfully\",\"is_error\":false}'\n    exit 0\n  fi\ndone\necho '{\"result\":\"Error\",\"is_error\":true}'\nexit 1\n").unwrap();
        let follow_up = make_message_json(chat_id, 100, "Are you there?");
        let res2 = handle_message(bot, follow_up, cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info).await;
        assert!(res2.is_ok(), "Second handle_message failed: {:?}", res2);
        let active2 = mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(active2.provider_sessions.get("antigravity").map(|p| p.message_count), Some(2));
    }

    #[tokio::test]
    async fn test_commands_routing_e2e() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let temp_dir = tempfile::tempdir().unwrap();
        mock_brain_dir(&temp_dir, "mock-session-123");
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, _env) = setup_e2e_env(&temp_dir, "#!/bin/sh\necho '{\"result\":\"dummy\",\"is_error\":false}'\n");
        for cmd in &["/status", "/memory", "/stop", "/model", "/cron"] {
            let msg = make_message_json(888, 100, cmd);
            assert!(handle_message(bot.clone(), msg, cfg.clone(), mgr.clone(), cli.clone(), cron_mgr.clone(), topic_cache.clone(), bot_info.clone()).await.is_ok());
        }
    }

    #[tokio::test]
    async fn test_directive_parsing_e2e() {
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let temp_dir = tempfile::tempdir().unwrap();
        mock_brain_dir(&temp_dir, "mock-session-123");
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, _env) = setup_e2e_env(&temp_dir, "#!/bin/sh\necho '{\"result\":\"directive response\",\"is_error\":false}'\n");
        let msg1 = make_message_json(777, 100, "@model opus Hello");
        let _ = handle_message(bot.clone(), msg1, cfg.clone(), mgr.clone(), cli.clone(), cron_mgr.clone(), topic_cache.clone(), bot_info.clone()).await;
        assert_eq!(mgr.get_active(&SessionKey::telegram(777, None)).await.unwrap().unwrap().model, "opus");

        let msg2 = make_message_json(777, 100, "@opus Hello again");
        let _ = handle_message(bot, msg2, cfg, mgr, cli, cron_mgr, topic_cache, bot_info).await;
    }
}
