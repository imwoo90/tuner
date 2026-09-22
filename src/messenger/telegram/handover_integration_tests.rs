//! # Topic Handover Integration Tests
//!
//! ## Overview
//! Validates end-to-end message routing and state transitions for `/new` topic handover:
//! 1. Existing forum topic with history compresses context and performs handover.
//! 2. Brand new forum topic message boots completely fresh without history.
//! 3. Existing topic without history falls back to fresh boot without error.
//! 4. The `/reset` command alias performs handover identically to `/new`.
//! 5. Bot username suffix `/new@bot` is cleanly supported.
//! 6. Consecutive `/new` calls preserve topic objective and decisions.

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
    use crate::messenger::telegram::history::log_telegram_message;
    use tempfile::NamedTempFile;

    type Setup = (
        Arc<SessionManager>,
        Arc<CliConfig>,
        Arc<AntigravityCli>,
        Bot,
        Arc<CronManager>,
        Arc<TopicNameCache>,
        Arc<BotInfo>,
        Arc<crate::telegram::media_group::MediaGroupManager>,
        tempfile::TempDir,
    );

    fn setup() -> Setup {
        let temp_ws = tempfile::tempdir().unwrap();
        let temp = NamedTempFile::new().unwrap();
        let mgr = Arc::new(SessionManager::new(temp.path().to_path_buf(), 30, 4, false, "UTC".to_string(), None));
        let cfg = Arc::new(CliConfig {
            provider: "antigravity".to_string(),
            allowed_user_ids: vec![100],
            allowed_group_ids: vec![123],
            working_dir: temp_ws.path().to_path_buf(),
            ..Default::default()
        });
        let cli = Arc::new(AntigravityCli::new((*cfg).clone()));
        let bot = Bot::new("123:abc");
        let temp_cron = NamedTempFile::new().unwrap();
        let cron_mgr = Arc::new(CronManager::new(temp_cron.path().to_path_buf()));
        let topic_cache = Arc::new(TopicNameCache::new());
        let bot_info = Arc::new(BotInfo { username: Some("my_bot".to_string()) });
        let mgm = Arc::new(crate::telegram::media_group::MediaGroupManager::new());
        (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm, temp_ws)
    }

    fn make_msg(cmd: &str, tid: i64) -> Message {
        let j = format!(
            r#"{{"message_id":100,"date":1,"chat":{{"id":123,"type":"supergroup","is_forum":true}},"message_thread_id":{},"from":{{"id":100,"is_bot":false,"first_name":"I"}},"text":"{}","is_topic_message":true}}"#,
            tid, cmd
        );
        serde_json::from_str(&j).unwrap()
    }

    fn seed_history(ws: &std::path::Path, sid: &str, tid: i64, tname: &str) {
        let t = Some(tname);
        log_telegram_message(ws, sid, Some(tid), t, "user", Some(1), "Implement OAuth2 authentication", true, None);
        log_telegram_message(ws, sid, Some(tid), t, "bot", Some(2), "Decided to use GitHub OAuth2 provider.", true, None);
        log_telegram_message(ws, sid, Some(tid), t, "user", Some(3), "Add CSRF token validation", true, None);
        log_telegram_message(ws, sid, Some(tid), t, "bot", Some(4), "CSRF token validation implemented and verified.", true, None);
    }

    async fn init_active_session(mgr: &SessionManager, key: &SessionKey, prov: &str, sid: &str) {
        let (sess, _) = mgr.resolve_session(key, prov, "opus").await.unwrap();
        let mut updated = sess.clone();
        updated.set_session_id(prov, sid);
        mgr.update_session(&updated, 0.0, 0).await.unwrap();
    }

    #[tokio::test]
    async fn test_new_command_in_topic_with_history_performs_handover() {
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm, _ws) = setup();
        let key = SessionKey::telegram(123, Some(555));
        topic_cache.insert(123, 555, "Auth Module".to_string());
        init_active_session(&mgr, &key, &cfg.provider, "old-sess-555").await;
        seed_history(&cfg.working_dir, "old-sess-555", 555, "Auth Module");

        handle_message(bot, make_msg("/new", 555), cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info, mgm).await.unwrap();

        let s = mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(s.get_session_id("antigravity"), "mock-session-123");
        let summary = s.last_handover_summary.unwrap();
        assert!(summary.contains("OAuth2"));
        assert!(summary.contains("GitHub OAuth2") || summary.contains("CSRF"));
    }

    #[tokio::test]
    async fn test_reset_command_alias_performs_handover() {
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm, _ws) = setup();
        let key = SessionKey::telegram(123, Some(777));
        topic_cache.insert(123, 777, "Feature X".to_string());
        init_active_session(&mgr, &key, &cfg.provider, "old-sess-777").await;
        seed_history(&cfg.working_dir, "old-sess-777", 777, "Feature X");

        handle_message(bot, make_msg("/reset", 777), cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info, mgm).await.unwrap();

        let s = mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(s.get_session_id("antigravity"), "mock-session-123");
        assert!(s.last_handover_summary.is_some());
    }

    #[tokio::test]
    async fn test_new_command_with_bot_username_alias_performs_handover() {
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm, _ws) = setup();
        let key = SessionKey::telegram(123, Some(666));
        topic_cache.insert(123, 666, "Bot Alias".to_string());
        init_active_session(&mgr, &key, &cfg.provider, "old-sess-666").await;
        seed_history(&cfg.working_dir, "old-sess-666", 666, "Bot Alias");

        handle_message(bot, make_msg("/new@my_bot", 666), cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info, mgm).await.unwrap();

        let s = mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(s.get_session_id("antigravity"), "mock-session-123");
        assert!(s.last_handover_summary.is_some());
    }

    #[tokio::test]
    async fn test_consecutive_new_commands_preserve_handover_context() {
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm, _ws) = setup();
        let key = SessionKey::telegram(123, Some(444));
        topic_cache.insert(123, 444, "Multi Reset".to_string());
        init_active_session(&mgr, &key, &cfg.provider, "sess-turn-1").await;
        seed_history(&cfg.working_dir, "sess-turn-1", 444, "Multi Reset");

        handle_message(bot.clone(), make_msg("/new", 444), cfg.clone(), mgr.clone(), cli.clone(), cron_mgr.clone(), topic_cache.clone(), bot_info.clone(), mgm.clone()).await.unwrap();

        handle_message(bot, make_msg("/new", 444), cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info, mgm).await.unwrap();

        let s = mgr.get_active(&key).await.unwrap().unwrap();
        let summary = s.last_handover_summary.expect("Must retain summary across consecutive /new");
        assert!(summary.contains("OAuth2"));
        assert!(summary.contains("GitHub OAuth2") || summary.contains("CSRF"));
    }

    #[tokio::test]
    async fn test_brand_new_topic_first_message_boots_fresh() {
        let (mgr, cfg, cli, bot, _cron_mgr, _topic_cache, _bot_info, _mgm, _ws) = setup();
        let key = SessionKey::telegram(123, Some(999));
        let (mut sess, _) = mgr.resolve_session(&key, &cfg.provider, "opus").await.unwrap();
        let active_sid = crate::messenger::telegram::session_init::initialize_session_if_needed(
            &bot, &make_msg("First greeting", 999), &mgr, &mut sess, &cli, &cfg,
        ).await.unwrap();

        assert_eq!(active_sid, "mock-session-123");
        let s = mgr.get_active(&key).await.unwrap().unwrap();
        assert!(s.last_handover_summary.is_none());
    }

    #[tokio::test]
    async fn test_new_command_in_topic_without_history_boots_fresh() {
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm, _ws) = setup();
        let key = SessionKey::telegram(123, Some(888));
        init_active_session(&mgr, &key, &cfg.provider, "no-history-sid").await;

        handle_message(bot, make_msg("/new", 888), cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info, mgm).await.unwrap();

        let s = mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(s.get_session_id("antigravity"), "mock-session-123");
        assert!(s.last_handover_summary.is_none());
    }
}
