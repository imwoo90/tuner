#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use teloxide::Bot;
    use crate::config::CliConfig;
    use crate::session::manager::SessionManager;
    use crate::cli::antigravity::AntigravityCli;
    use crate::cron::manager::CronManager;
    use crate::messenger::telegram::topic_cache::{BotInfo, TopicNameCache};
    use crate::messenger::telegram::commands_registry::is_lock_free_command;
    use crate::messenger::telegram::handler::handle_message;

    #[test]
    fn test_is_lock_free_command_classification() {
        // Lock-free commands: read-only info, diagnostics, and process interrupts
        assert!(is_lock_free_command("/status"));
        assert!(is_lock_free_command("/help"));
        assert!(is_lock_free_command("/start"));
        assert!(is_lock_free_command("/diagnose"));
        assert!(is_lock_free_command("/memory"));
        assert!(is_lock_free_command("/cron"));
        assert!(is_lock_free_command("/upgrade"));
        assert!(is_lock_free_command("/restart"));
        assert!(is_lock_free_command("/stop"));
        assert!(is_lock_free_command("/stop_all"));
        assert!(is_lock_free_command("/abort"));

        // Commands that mutate session state and MUST acquire topic lock
        assert!(!is_lock_free_command("/new"));
        assert!(!is_lock_free_command("/reset"));
        assert!(!is_lock_free_command("/model"));
        assert!(!is_lock_free_command("/effort"));
        assert!(!is_lock_free_command("/lang"));

        // Regular chat text
        assert!(!is_lock_free_command("Hello assistant!"));
        assert!(!is_lock_free_command("Please help me debug this"));
    }

    fn setup() -> (
        Arc<SessionManager>,
        Arc<CliConfig>,
        Arc<AntigravityCli>,
        Bot,
        Arc<CronManager>,
        Arc<TopicNameCache>,
        Arc<BotInfo>,
        Arc<crate::messenger::telegram::media_group::MediaGroupManager>,
    ) {
        let temp_sess = tempfile::NamedTempFile::new().unwrap();
        let mgr = Arc::new(SessionManager::new(temp_sess.path().to_path_buf(), 30, 4, false, "UTC".to_string(), None));
        let cfg = Arc::new(CliConfig {
            provider: "antigravity".to_string(),
            allowed_user_ids: vec![100],
            allowed_group_ids: vec![123],
            ..Default::default()
        });
        let cli = Arc::new(AntigravityCli::new((*cfg).clone()));
        let bot = Bot::new("12345:mock-token-test");
        let cron_file = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        let cron_mgr = Arc::new(CronManager::new(cron_file.to_path_buf()));
        let topic_cache = Arc::new(TopicNameCache::new());
        let bot_info = Arc::new(BotInfo { username: Some("tuner_bot".to_string()) });
        let mgm = Arc::new(crate::messenger::telegram::media_group::MediaGroupManager::new());
        (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm)
    }

    #[tokio::test]
    async fn test_status_bypasses_topic_lock() {
        let (mgr, cfg_arc, cli, bot, cron_mgr, topic_cache, bot_info, mgm) = setup();
        let lock = mgr.lock_pool.get((123, Some(456)));
        let _held_guard = lock.lock().await;

        let json = r#"{"message_id":901,"date":1,"chat":{"id":123,"type":"supergroup","is_forum":true},"from":{"id":100,"is_bot":false,"first_name":"Tester"},"text":"/status","message_thread_id":456,"is_topic_message":true}"#;
        let msg: teloxide::types::Message = serde_json::from_str(json).unwrap();
        let res = handle_message(bot, msg, cfg_arc, mgr, cli, cron_mgr, topic_cache, bot_info, mgm).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_stop_bypasses_topic_lock() {
        let (mgr, cfg_arc, cli, bot, cron_mgr, topic_cache, bot_info, mgm) = setup();
        let lock = mgr.lock_pool.get((123, Some(456)));
        let _held_guard = lock.lock().await;

        let json = r#"{"message_id":902,"date":1,"chat":{"id":123,"type":"supergroup","is_forum":true},"from":{"id":100,"is_bot":false,"first_name":"Tester"},"text":"/stop","message_thread_id":456,"is_topic_message":true}"#;
        let msg: teloxide::types::Message = serde_json::from_str(json).unwrap();
        let res = handle_message(bot, msg, cfg_arc, mgr, cli, cron_mgr, topic_cache, bot_info, mgm).await;
        assert!(res.is_ok());
    }
}
