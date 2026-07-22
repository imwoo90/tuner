#[cfg(test)]
mod tests {
    use crate::telegram::handle_message;
    use crate::config::CliConfig;
    use crate::cli::antigravity::AntigravityCli;
    use crate::session::key::SessionKey;
    use crate::session::manager::SessionManager;
    use crate::cron::manager::CronManager;
    use std::sync::Arc;
    use teloxide::Bot;
    use teloxide::types::Message;
    use crate::telegram::{TopicNameCache, BotInfo};

    fn setup() -> (
        Arc<SessionManager>,
        Arc<CliConfig>,
        Arc<AntigravityCli>,
        Bot,
        Arc<CronManager>,
        Arc<TopicNameCache>,
        Arc<BotInfo>,
        Arc<crate::telegram::media_group::MediaGroupManager>
    ) {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mgr = Arc::new(SessionManager::new(temp.path().to_path_buf(), 30, 4, false, "UTC".to_string(), None));
        let cfg = Arc::new(CliConfig {
            provider: "antigravity".to_string(),
            allowed_user_ids: vec![100],
            allowed_group_ids: vec![123],
            ..Default::default()
        });
        let cli = Arc::new(AntigravityCli::new((*cfg).clone()));
        let bot = Bot::new("123:abc");
        let temp_cron = tempfile::NamedTempFile::new().unwrap();
        let cron_mgr = Arc::new(CronManager::new(temp_cron.path().to_path_buf()));
        let topic_cache = Arc::new(TopicNameCache::new());
        let bot_info = Arc::new(BotInfo { username: Some("my_bot".to_string()) });
        let mgm = Arc::new(crate::telegram::media_group::MediaGroupManager::new());
        (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm)
    }

    fn make_msg(json: &str) -> Message {
        serde_json::from_str(json).unwrap()
    }

    #[tokio::test]
    async fn test_telegram_forum_topic_events() {
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm) = setup();
        let msg_created = make_msg(r#"{"message_id":13,"date":1,"chat":{"id":123,"type":"supergroup"},"from":{"id":100,"is_bot":false},"forum_topic_created":{"name":"QA Thread","icon_color":0},"message_thread_id":999}"#);
        handle_message(bot.clone(), msg_created, cfg.clone(), mgr.clone(), cli.clone(), cron_mgr.clone(), topic_cache.clone(), bot_info.clone(), mgm.clone()).await.unwrap();

        // Verify it is inserted in topic_cache
        assert_eq!(topic_cache.find_by_name(123, "@QA Thread"), Some(999));

        let msg_edited = make_msg(r#"{"message_id":14,"date":1,"chat":{"id":123,"type":"supergroup"},"from":{"id":100,"is_bot":false},"forum_topic_edited":{"name":"QA & Testing Thread"},"message_thread_id":999}"#);
        handle_message(bot.clone(), msg_edited, cfg.clone(), mgr.clone(), cli.clone(), cron_mgr.clone(), topic_cache.clone(), bot_info.clone(), mgm.clone()).await.unwrap();

        // Verify it is updated in topic_cache
        assert_eq!(topic_cache.find_by_name(123, "@QA & Testing Thread"), Some(999));
    }

    #[tokio::test]
    async fn test_telegram_commands_specification() {
        let commands = crate::telegram::commands::get_bot_commands();
        assert!(!commands.is_empty());
        for cmd in &commands {
            assert!(!cmd.command.is_empty() && cmd.command.len() <= 32);
            for c in cmd.command.chars() {
                assert!(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            }
            assert!(!cmd.description.is_empty() && cmd.description.len() <= 256);
        }
        let names: Vec<_> = commands.iter().map(|c| c.command.as_str()).collect();
        for n in &["new", "reset", "stop", "model", "plan", "grill_me", "goal", "learn", "teamwork_preview"] {
            assert!(names.contains(n));
        }
        assert!(!names.contains(&"diagnose"));
    }
}
