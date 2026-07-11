//! # Telegram Integration Tests
//!
//! This module contains unit and integration tests for the Telegram bot interface,
//! validating message reply prompt building and SessionManager integration.

#[cfg(test)]
mod tests {
    use crate::telegram::handle_message;
    use crate::config::CliConfig;
    use crate::cli::antigravity::AntigravityCli;
    use crate::session::key::SessionKey;
    use crate::session::manager::SessionManager;
    use std::sync::Arc;
    use teloxide::Bot;
    use teloxide::types::Message;

    #[tokio::test]
    async fn test_telegram_session_manager_integration() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let sessions_mgr = Arc::new(SessionManager::new(
            temp.path().to_path_buf(),
            30,
            4,
            false,
            "UTC".to_string(),
            None,
        ));
        
        let config = Arc::new(CliConfig {
            provider: "antigravity".to_string(),
            allowed_user_ids: vec![100],
            ..Default::default()
        });

        let key = SessionKey::telegram(123, None);
        let (sess, is_new) = sessions_mgr.resolve_session(&key, &config.provider, "opus").await.unwrap();
        assert!(is_new);
        assert_eq!(sess.chat_id, 123);
        assert_eq!(sess.provider, "antigravity");

        let cli = Arc::new(AntigravityCli::new((*config).clone()));
        let bot = Bot::new("123:abc");
        let msg_json = r#"{
            "message_id": 1,
            "date": 123456,
            "chat": {"id": 123, "type": "private"},
            "from": {"id": 100, "is_bot": false, "first_name": "InMyung", "username": "inmyung"},
            "text": "hello"
        }"#;
        let msg: Message = serde_json::from_str(msg_json).unwrap();
        
        let temp_cron = tempfile::NamedTempFile::new().unwrap();
        let cron_mgr = Arc::new(crate::cron::manager::CronManager::new(temp_cron.path().to_path_buf()));
        let topic_cache = Arc::new(crate::telegram::TopicNameCache::new());
        let bot_info = Arc::new(crate::telegram::BotInfo { username: Some("my_bot".to_string()) });

        let res = handle_message(bot, msg, config, sessions_mgr, cli, cron_mgr, topic_cache, bot_info).await;
        assert!(res.is_ok());
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
        let json = r#"{"message_id":2,"date":123457,"chat":{"id":123,"type":"group","title":"G"},"from":{"id":100,"is_bot":false,"first_name":"I","username":"u"},"migrate_to_chat_id":456}"#;
        let msg: Message = serde_json::from_str(json).unwrap();

        let temp_cron = tempfile::NamedTempFile::new().unwrap();
        let cron_mgr = Arc::new(crate::cron::manager::CronManager::new(temp_cron.path().to_path_buf()));
        let topic_cache = Arc::new(crate::telegram::TopicNameCache::new());
        let bot_info = Arc::new(crate::telegram::BotInfo { username: Some("my_bot".to_string()) });

        handle_message(bot, msg, cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info).await.unwrap();

        let new_key = SessionKey::telegram(456, None);
        let new_sess = mgr.get_active(&new_key).await.unwrap().unwrap();
        assert_eq!(new_sess.chat_id, 456);
        assert_eq!(new_sess.get_session_id("antigravity"), "migrated-session-123");
        assert!(mgr.get_active(&old_key).await.unwrap().is_none());
    }
}
