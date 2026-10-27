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
    use crate::config::CliConfig;
    use crate::cli::antigravity::AntigravityCli;
    use crate::session::key::SessionKey;
    use crate::session::manager::SessionManager;
    use crate::cron::manager::CronManager;
    use std::sync::Arc;
    use teloxide::Bot;
    use teloxide::types::Message;
    use crate::telegram::{TopicNameCache, BotInfo};
    use tempfile::NamedTempFile;

    #[allow(clippy::too_many_arguments)]
    fn log_telegram_message(
        working_dir: &std::path::Path,
        session_id: &str,
        topic_id: Option<i64>,
        topic_name: Option<&str>,
        sender: &str,
        message_id: Option<i32>,
        text: &str,
        is_success: bool,
        error: Option<&str>,
    ) {
        crate::messenger::telegram::history::log_telegram_message(
            working_dir,
            session_id,
            (topic_id, topic_name),
            sender,
            message_id,
            text,
            (is_success, error),
        );
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_message(
        bot: Bot,
        msg: Message,
        config: Arc<CliConfig>,
        sessions: Arc<SessionManager>,
        cli: Arc<AntigravityCli>,
        cron_manager: Arc<CronManager>,
        topic_cache: Arc<TopicNameCache>,
        bot_info: Arc<BotInfo>,
        media_group_manager: Arc<crate::telegram::media_group::MediaGroupManager>,
    ) -> Result<(), teloxide::RequestError> {
        crate::telegram::handle_message(
            bot,
            msg,
            config,
            sessions,
            cli,
            (cron_manager, topic_cache),
            (bot_info, media_group_manager),
        )
        .await
    }

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
        mgr.update_session(&updated).await.unwrap();
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

        // Log follow-up activity in the newly created session ("mock-session-123") without repeating "OAuth2"
        log_telegram_message(&cfg.working_dir, "mock-session-123", Some(444), Some("Multi Reset"), "user", Some(5), "Add unit tests for login handler", true, None);
        log_telegram_message(&cfg.working_dir, "mock-session-123", Some(444), Some("Multi Reset"), "bot", Some(6), "Unit tests added and passing.", true, None);

        handle_message(bot, make_msg("/new", 444), cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info, mgm).await.unwrap();

        let s = mgr.get_active(&key).await.unwrap().unwrap();
        let summary = s.last_handover_summary.expect("Must retain summary across consecutive /new");
        assert!(summary.contains("OAuth2"));
        assert!(summary.contains("GitHub OAuth2"));
        assert!(summary.contains("Unit tests added"));
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

    #[tokio::test]
    async fn test_auto_handover_on_expired_session_and_model_preservation() {
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm, _ws) = setup();
        let key = SessionKey::telegram(123, Some(333));
        topic_cache.insert(123, 333, "Auto Expiry".to_string());
        init_active_session(&mgr, &key, &cfg.provider, "expired-sess-333").await;
        seed_history(&cfg.working_dir, "expired-sess-333", 333, "Auto Expiry");

        // Simulate idle/daily expiration rotating session_id into past_session_ids
        let mut expired = mgr.get_active(&key).await.unwrap().unwrap();
        expired.model = "claude-opus-5-5".to_string();
        expired.clear_provider_session("antigravity");
        mgr.preserve_session_identity(&expired).await.unwrap();

        let (mut sess, _) = mgr.resolve_session(&key, &cfg.provider, "gemini-3.8-flash").await.unwrap();
        let active_sid = crate::messenger::telegram::session_init::initialize_session_if_needed(
            &bot, &make_msg("Resume after expiry", 333), &mgr, &mut sess, &cli, &cfg,
        ).await.unwrap();
        assert_eq!(active_sid, "mock-session-123");

        let s_after = mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(s_after.model, "claude-opus-5-5");
        let summary = s_after.last_handover_summary.expect("Auto-expiry must populate handover summary");
        assert!(summary.contains("OAuth2"));

        // Verify /new also preserves the topic model ("claude-opus-5-5")
        handle_message(bot, make_msg("/new", 333), cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info, mgm).await.unwrap();
        let s_new = mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(s_new.model, "claude-opus-5-5");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_new_command_with_inline_prompt_resets_and_preserves_handover() {
        let _guard = crate::telegram::TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let temp_dir = tempfile::tempdir().unwrap();
        crate::telegram::test_helpers::helpers::mock_brain_dir(&temp_dir, "mock-session-123");
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm, _env) =
            crate::telegram::test_helpers::helpers::setup_e2e_env(
                &temp_dir,
                "#!/bin/sh\necho '{\"result\":\"ok\",\"is_error\":false}'\n",
            );
        let key = SessionKey::telegram(123, Some(222));
        topic_cache.insert(123, 222, "Inline New".to_string());
        init_active_session(&mgr, &key, &cfg.provider, "old-sess-222").await;
        seed_history(&cfg.working_dir, "old-sess-222", 222, "Inline New");

        handle_message(bot, make_msg("/new Implement step 2 now", 222), cfg.clone(), mgr.clone(), cli, cron_mgr, topic_cache, bot_info, mgm).await.unwrap();

        let s = mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(s.get_session_id("antigravity"), "mock-session-123");
        assert_eq!(s.get_past_session_ids("antigravity"), vec!["old-sess-222".to_string()]);
        assert_eq!(s.topic_name.as_deref(), Some("Inline New"));
        assert!(s.last_handover_summary.as_deref().unwrap_or("").contains("OAuth2"));
        let entries = crate::messenger::telegram::handover_helpers::extract_topic_history(&cfg.working_dir, "mock-session-123");
        assert!(entries.iter().any(|e| e.sender == "user" && e.text == "Implement step 2 now"));
    }

    #[tokio::test]
    async fn test_new_command_in_general_dm_performs_handover_and_unit_helpers() {
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm, _ws) = setup();
        let key = SessionKey::telegram(123, None);
        init_active_session(&mgr, &key, &cfg.provider, "dm-sess-1").await;
        log_telegram_message(&cfg.working_dir, "dm-sess-1", None, None, "user", Some(1), "Build DM workflow", true, None);
        log_telegram_message(&cfg.working_dir, "dm-sess-1", None, None, "bot", Some(2), "Agreed to use SQLite.", true, None);

        let dm_msg: Message = serde_json::from_str(
            r#"{"message_id":101,"date":1,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"text":"/new"}"#,
        ).unwrap();
        handle_message(bot, dm_msg, cfg, mgr.clone(), cli, cron_mgr, topic_cache.clone(), bot_info, mgm).await.unwrap();
        let s = mgr.get_active(&key).await.unwrap().unwrap();
        assert!(s.last_handover_summary.as_deref().unwrap_or("").contains("Build DM workflow"));

        // Verify parse_new_command_args supports multi-word cached topic names vs inline prompts
        topic_cache.insert(123, 901, "Auth Module".to_string());
        let m = make_msg("/new", 555);
        assert_eq!(
            crate::messenger::telegram::commands_session::parse_new_command_args(&m, "/new @Auth Module", &topic_cache, Some(555)),
            (Some(901), None)
        );
        assert_eq!(
            crate::messenger::telegram::commands_session::parse_new_command_args(&m, "/new Refactor auth module", &topic_cache, Some(555)),
            (Some(555), Some("Refactor auth module"))
        );

        // Verify strip_new_or_reset_prefix and should_send_fresh_completion_message
        assert_eq!(crate::messenger::telegram::strip_new_or_reset_prefix(" /new@my_bot Run tests "), "Run tests");
        assert_eq!(crate::messenger::telegram::strip_new_or_reset_prefix("Normal prompt"), "Normal prompt");

        let mut resp = crate::cli::CliResponse {
            result: "Done".to_string(),
            is_error: false,
            session_id: Some("s1".to_string()),
            returncode: Some(0),
            stderr: String::new(),
        };
        assert!(!crate::messenger::telegram::stream::should_send_fresh_completion_message(&resp, std::time::Duration::from_secs(30)));
        assert!(crate::messenger::telegram::stream::should_send_fresh_completion_message(&resp, std::time::Duration::from_secs(60)));
        resp.stderr = "Interrupted by user (/stop)".to_string();
        assert!(!crate::messenger::telegram::stream::should_send_fresh_completion_message(&resp, std::time::Duration::from_secs(90)));
        resp.stderr.clear();
        resp.result.clear();
        assert!(!crate::messenger::telegram::stream::should_send_fresh_completion_message(&resp, std::time::Duration::from_secs(90)));
        resp.is_error = true;
        assert!(crate::messenger::telegram::stream::should_send_fresh_completion_message(&resp, std::time::Duration::from_secs(90)));
    }
}
