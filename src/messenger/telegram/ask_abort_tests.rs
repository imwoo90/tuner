#![allow(clippy::await_holding_lock)]
#[cfg(test)]
mod tests {
    use crate::telegram::{handle_message, TopicNameCache, BotInfo, TEST_ENV_MUTEX};
    use crate::telegram::test_helpers::helpers::{EnvGuard, mock_brain_dir, make_msg};
    use crate::config::CliConfig;
    use crate::cli::antigravity::AntigravityCli;
    use crate::session::key::SessionKey;
    use crate::session::manager::SessionManager;
    use crate::cron::manager::CronManager;
    use std::sync::Arc;
    use teloxide::Bot;
    use teloxide::types::Message;

    fn make_message_json(chat_id: i64, user_id: i64, text: &str) -> Message {
        let raw = format!(
            r#"{{"message_id":100,"date":1,"chat":{{"id":{},"type":"private"}},"from":{{"id":{},"is_bot":false,"first_name":"User"}},"text":"{}"}}"#,
            chat_id, user_id, text
        );
        make_msg(&raw)
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
    async fn test_telegram_active_ask_feeds_on_new_message() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
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
        ctx.mgr.update_session(&updated).await.unwrap();
        let mut env = std::collections::HashMap::new();
        env.insert("TUNER_CHAT_ID".to_string(), chat_id.to_string());
        ctx.cli.sessions.ensure_session(session_id, temp_dir.path(), "sleep", &["10".to_string()], &env).await.unwrap();
        assert!(ctx.cli.sessions.is_active(session_id).await);
        ctx.cli.sessions.set_running(session_id, true).await;
        ctx.cli.sessions.set_ask_active(session_id, true).await;
        assert!(ctx.cli.sessions.is_ask_active(session_id).await);
        let msg = make_message_json(chat_id, 100, "Answer to active ask");
        let mgm = Arc::new(crate::telegram::media_group::MediaGroupManager::new());
        let res = handle_message(ctx.bot, msg, ctx.cfg, ctx.mgr, ctx.cli.clone(), (ctx.cron_mgr, ctx.topic_cache), (ctx.bot_info, mgm)).await;
        assert!(res.is_ok());
        assert!(ctx.cli.sessions.is_active(session_id).await);
        assert!(!ctx.cli.sessions.is_ask_active(session_id).await);
    }

    #[tokio::test]
    async fn test_feed_active_session_waits_for_running_turn_and_returns_false() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let temp_dir = tempfile::tempdir().unwrap();
        let session_id = "test-session-running-turn";
        mock_brain_dir(&temp_dir, session_id);
        let _env = EnvGuard::new(&temp_dir);
        let ctx = init_test_context(&temp_dir);
        let chat_id = 999;
        let mut env = std::collections::HashMap::new();
        env.insert("TUNER_CHAT_ID".to_string(), chat_id.to_string());
        ctx.cli.sessions.ensure_session(session_id, temp_dir.path(), "sleep", &["10".to_string()], &env).await.unwrap();
        ctx.cli.sessions.set_running(session_id, true).await;

        let sessions_clone = ctx.cli.sessions.clone();
        let sid_clone = session_id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            sessions_clone.set_running(&sid_clone, false).await;
        });

        let msg = make_message_json(chat_id, 100, "New message while running");
        let start = std::time::Instant::now();
        let handled = crate::messenger::telegram::ask_helpers::feed_active_session_if_running(
            &ctx.bot,
            &msg,
            session_id,
            "New message while running",
            &ctx.cli,
            (&ctx.mgr, crate::session::data::SessionData::default()),
            &ctx.cfg,
        ).await.unwrap();

        assert!(!handled);
        assert!(start.elapsed().as_millis() >= 150);
    }

    #[tokio::test]
    async fn test_defect01_active_ask_protected_and_cleaned_on_terminate() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::new(&temp_dir);
        let ctx = init_test_context(&temp_dir);
        let (chat_id, sid) = (777, "ask-defect-01");
        let key = SessionKey::telegram(chat_id, None);
        let (mut sess, _) = ctx.mgr.resolve_session(&key, "antigravity", "gemini-4-argon").await.unwrap();
        sess.set_session_id("antigravity", sid);
        ctx.mgr.preserve_session_identity(&sess).await.unwrap();

        let mut env = std::collections::HashMap::new();
        env.insert("TUNER_CHAT_ID".to_string(), chat_id.to_string());
        ctx.cli.sessions.ensure_session(sid, temp_dir.path(), "sleep", &["10".to_string()], &env).await.unwrap();
        ctx.cli.sessions.holders.lock().await.get_mut(sid).unwrap().initialized = true;
        ctx.cli.sessions.set_ask_active(sid, true).await;

        sess.model = "antigravity-default".to_string();
        ctx.mgr.preserve_session_identity(&sess).await.unwrap();
        crate::messenger::telegram::commands_model::apply_model_to_active_pty(&ctx.cli, &ctx.mgr, &sess).await;
        assert!(ctx.cli.sessions.is_active(sid).await, "Open ask PTY must not be killed by antigravity-default switch");
        assert!(ctx.cli.sessions.is_ask_active(sid).await);

        ctx.cli.terminate_session_clean(sid).await;
        assert!(!ctx.cli.sessions.is_active(sid).await);
        assert!(!ctx.cli.sessions.is_ask_active(sid).await);
        assert!(ctx.cli.sessions.active_asks.lock().await.is_empty());
    }

    #[tokio::test]
    async fn test_defect02_topic_turn_guard_defers_live_pty_sync_until_idle() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::new(&temp_dir);
        let ctx = init_test_context(&temp_dir);
        let (chat_id, topic_id, sid) = (778, Some(42), "turn-guard-02");
        let key = SessionKey::telegram(chat_id, topic_id);
        let (mut sess, _) = ctx.mgr.resolve_session(&key, "antigravity", "gemini-4-argon").await.unwrap();
        sess.set_session_id("antigravity", sid);
        sess.model = "gemini-pro-orchestrator".to_string();
        sess.effort = None;
        ctx.mgr.preserve_session_identity(&sess).await.unwrap();

        let mut env = std::collections::HashMap::new();
        env.insert("TUNER_CHAT_ID".to_string(), chat_id.to_string());
        ctx.cli.sessions.ensure_session(sid, temp_dir.path(), "sleep", &["10".to_string()], &env).await.unwrap();
        let pid = {
            let mut hs = ctx.cli.sessions.holders.lock().await;
            let h = hs.get_mut(sid).unwrap();
            h.initialized = true;
            h.child.id().unwrap()
        };

        let turn_guard = ctx.cli.begin_topic_turn(chat_id, topic_id);
        assert!(ctx.cli.is_pty_busy_or_asking(sid, chat_id, topic_id).await);
        crate::messenger::telegram::commands_model::apply_model_to_active_pty(&ctx.cli, &ctx.mgr, &sess).await;
        assert_eq!(ctx.cli.pty_sync.get_applied(sid, pid).await, None);

        drop(turn_guard);
        assert!(!ctx.cli.is_topic_turn_in_flight(chat_id, topic_id));
        crate::messenger::telegram::commands_model::apply_model_to_active_pty(&ctx.cli, &ctx.mgr, &sess).await;
        assert_eq!(ctx.cli.pty_sync.get_applied(sid, pid).await.as_deref(), Some("gemini-pro-orchestrator"));
        ctx.cli.terminate_session_clean(sid).await;
    }

    #[tokio::test]
    async fn test_defect03_init_locked_pty_sets_initialized_and_applied_atomically() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::new(&temp_dir);
        let mut cfg = CliConfig {
            provider: "antigravity".to_string(),
            model: Some("gemini-pro-orchestrator".to_string()),
            working_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };
        cfg.effort = Some("high".to_string());
        let cli = AntigravityCli::new(cfg);
        let sid = "pty-init-03";
        let env = cli.build_env();
        cli.sessions
            .ensure_session(
                sid,
                temp_dir.path(),
                "sh",
                &["-c".to_string(), "printf '? for shortcuts  gemini-4-argon-high\\n'; sleep 10".to_string()],
                &env,
            )
            .await
            .unwrap();
        assert!(!cli.sessions.holders.lock().await.get(sid).unwrap().initialized);
        let lock = cli.pty_sync.lock_for(sid).await;
        let _g = lock.lock().await;
        let (pid, applied) = cli.init_locked_pty(sid).await.unwrap().unwrap();
        drop(_g);
        assert!(cli.sessions.holders.lock().await.get(sid).unwrap().initialized);
        assert_eq!(applied, None);
        assert_eq!(cli.pty_sync.get_applied(sid, pid).await, None);
        cli.terminate_session_clean(sid).await;
    }

    #[test]
    fn test_minor01_unicode_whitespace_and_minor02_provider_catalog_guard() {
        use crate::messenger::telegram::callbacks_effort::is_model_in_provider_catalog;
        use crate::messenger::telegram::reply_directives::{DirectiveOutcome, resolve_directive_for_provider};

        assert_eq!(
            resolve_directive_for_provider(
                "antigravity",
                None,
                "@model\u{00A0}  --effort\u{00A0} \u{3000}high\u{00A0}  carbon\u{00A0} \u{3000}fix unicode bug",
            ),
            DirectiveOutcome::Resolved("gemini-4-carbon".to_string(), Some("high".to_string()), "fix unicode bug")
        );
        assert_eq!(
            resolve_directive_for_provider("antigravity", None, "@\u{3000}carbon hello"),
            DirectiveOutcome::None
        );

        let agy_cat = crate::cli::antigravity::model_spec::fallback_models();
        let claude_cat = crate::cli::claude::discover_models();
        let codex_cat = crate::cli::codex::discover_models();

        assert!(is_model_in_provider_catalog("gemini-pro-orchestrator", &agy_cat));
        assert!(is_model_in_provider_catalog("antigravity-default", &agy_cat));
        assert!(!is_model_in_provider_catalog("gemini-pro-orchestrator", &claude_cat));
        assert!(!is_model_in_provider_catalog("antigravity-default", &claude_cat));
        assert!(is_model_in_provider_catalog("claude-fable-5", &claude_cat));
        assert!(!is_model_in_provider_catalog("claude-fable-5", &agy_cat));
        assert!(is_model_in_provider_catalog("gpt-6-astra", &codex_cat));
        assert!(!is_model_in_provider_catalog("gpt-oss-120b-medium", &codex_cat));
    }

    #[tokio::test]
    async fn test_round6_stream_persist_preserves_reset_and_peer_sessions() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::new(&temp_dir);
        let ctx = init_test_context(&temp_dir);
        let key = SessionKey::telegram(881, Some(12));
        let (mut old_snap, _) = ctx.mgr.resolve_session(&key, "claude", "claude-fable-5").await.unwrap();
        old_snap.set_session_id("claude", "old-claude-sid");
        old_snap.topic_name = Some("Topic Alpha".to_string());
        ctx.mgr.update_session(&old_snap).await.unwrap();

        let mut fresh = ctx.mgr.reset_provider_session(&key, "claude", "claude-fable-5").await.unwrap();
        fresh.set_session_id("claude", "fresh-after-new");
        fresh.set_session_id("codex", "peer-codex-sid");
        ctx.mgr.update_session(&fresh).await.unwrap();

        crate::messenger::telegram::stream::persist_stream_session_update(
            &ctx.mgr,
            &old_snap,
            old_snap.clone(),
            Some("old-claude-sid".to_string()),
            &ctx.cfg,
        ).await;

        let after = ctx.mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(after.get_session_id("claude"), "fresh-after-new");
        assert_eq!(after.get_session_id("codex"), "peer-codex-sid");
        assert_eq!(after.topic_name.as_deref(), Some("Topic Alpha"));
    }

    #[tokio::test]
    async fn test_round6_dead_ask_pruned_and_sid_locks_autopruned() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::new(&temp_dir);
        let ctx = init_test_context(&temp_dir);
        let (chat_id, sid) = (882, "dead-ask-sid");
        let key = SessionKey::telegram(chat_id, None);
        let (mut sess, _) = ctx.mgr.resolve_session(&key, "antigravity", "gemini-4-argon").await.unwrap();
        sess.set_session_id("antigravity", sid);
        ctx.mgr.update_session(&sess).await.unwrap();

        ctx.cli.sessions.set_ask_active(sid, true).await;
        assert!(!ctx.cli.is_pty_busy_or_asking(sid, chat_id, None).await);
        assert!(!ctx.cli.sessions.is_ask_active(sid).await);

        ctx.cli.sessions.set_ask_active("orphan-ask-upgrade", true).await;
        let status = crate::upgrade::evaluate_active_work(&ctx.cli).await;
        assert!(!status.is_busy);
        assert!(!ctx.cli.sessions.is_ask_active("orphan-ask-upgrade").await);

        {
            let _l1 = ctx.cli.pty_sync.lock_for("ephemeral-1").await;
            let _l2 = ctx.cli.pty_sync.lock_for("ephemeral-2").await;
            assert_eq!(ctx.cli.pty_sync.sid_lock_entry_count().await, 2);
        }
        assert_eq!(ctx.cli.pty_sync.sid_lock_entry_count().await, 0);

        let mut utf8_sess = crate::session::data::SessionData::default();
        utf8_sess.set_session_id("claude", "세션아이디123456789");
        let sw = utf8_sess.apply_provider_switch("claude");
        assert!(sw.contains("세션아이디123"));
    }

    #[tokio::test]
    async fn test_round6_heartbeat_busy_during_open_ask_and_turn_guard() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::new(&temp_dir);
        let ctx = init_test_context(&temp_dir);
        let (chat_id, topic_id, sid) = (883, Some(7), "mock-hb-ask-sid");
        let key = SessionKey::telegram(chat_id, topic_id);
        let (mut sess, _) = ctx.mgr.resolve_session(&key, "claude", "claude-fable-5").await.unwrap();
        sess.set_session_id("claude", sid);
        ctx.mgr.update_session(&sess).await.unwrap();

        let bus = Arc::new(crate::bus::bus::MessageBus::new());
        let hb = crate::heartbeat::scheduler::HeartbeatScheduler::new(ctx.cfg.clone(), ctx.mgr.clone(), ctx.cli.clone(), bus);
        assert!(!hb.is_session_busy(chat_id, topic_id).await);

        ctx.cli.sessions.set_ask_active(sid, true).await;
        assert!(hb.is_session_busy(chat_id, topic_id).await);
        ctx.cli.sessions.set_ask_active(sid, false).await;

        let guard = ctx.cli.begin_topic_turn(chat_id, topic_id);
        assert!(hb.is_session_busy(chat_id, topic_id).await);
        drop(guard);
        assert!(!hb.is_session_busy(chat_id, topic_id).await);
    }

    #[tokio::test]
    async fn test_round7_defect01_observer_ask_and_async_turn_provider_isolation() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::new(&temp_dir);
        let mut ctx = init_test_context(&temp_dir);
        ctx.cfg = Arc::new(CliConfig {
            provider: "claude".to_string(),
            allowed_user_ids: vec![100],
            working_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        });
        let (chat_id, topic_id) = (891, Some(21));
        let key = SessionKey::telegram(chat_id, topic_id);
        let (mut sess, _) = ctx.mgr.resolve_session(&key, "antigravity", "gemini-4-argon").await.unwrap();
        sess.set_session_id("antigravity", "agy-bg-1");
        sess.apply_provider_switch("claude");
        sess.set_session_id("claude", "claude-fg-1");
        sess.language = Some("ko".to_string());
        ctx.mgr.preserve_session_identity(&sess).await.unwrap();

        ctx.cli.sessions.set_ask_active("agy-bg-1", true).await;
        let obs_sess = crate::messenger::telegram::async_observer::build_observer_ask_session(
            ctx.mgr.get_active(&key).await.unwrap(),
            chat_id,
            topic_id,
            "agy-bg-1",
        );
        assert_eq!(obs_sess.provider, "antigravity");
        assert_eq!(obs_sess.get_session_id("antigravity"), "agy-bg-1");

        let ask = vec![crate::cli::AskQuestionData {
            question: "Stale agy question?".to_string(),
            options: vec!["Yes".to_string(), "No".to_string()],
            is_multi_select: false,
        }];
        crate::messenger::telegram::ask_process::handle_ask_question_event(
            &ctx.bot,
            teloxide::types::ChatId(chat_id),
            Some(21),
            ask.clone(),
            &obs_sess,
            (&ctx.cfg, &ctx.cli, &ctx.mgr),
        )
        .await
        .unwrap();
        assert!(!ctx.cli.sessions.is_ask_active("agy-bg-1").await);
        assert!(ctx.cli.sessions.get_ask_state("claude-fg-1").await.is_none());
        assert!(ctx.cli.sessions.get_ask_state("agy-bg-1").await.is_none());

        crate::messenger::telegram::async_observer::persist_async_antigravity_turn(
            &ctx.mgr,
            &key,
            "agy-bg-1",
            &ctx.cfg.provider,
        )
        .await;
        let after = ctx.mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(after.provider, "claude");
        assert_eq!(after.language.as_deref(), Some("ko"));
        assert_eq!(after.provider_sessions["antigravity"].message_count, 1);
        assert_eq!(after.provider_sessions["claude"].message_count, 0);

        // Post-/new on antigravity when global config.provider == "claude"
        let mut agy_new = after.clone();
        agy_new.apply_provider_switch("antigravity");
        agy_new.set_session_id("antigravity", "agy-fg-2");
        ctx.mgr.preserve_session_identity(&agy_new).await.unwrap();
        ctx.cli.sessions.set_ask_active("agy-bg-1", true).await;
        let obs_old = crate::messenger::telegram::async_observer::build_observer_ask_session(
            ctx.mgr.get_active(&key).await.unwrap(),
            chat_id,
            topic_id,
            "agy-bg-1",
        );
        crate::messenger::telegram::ask_process::handle_ask_question_event(
            &ctx.bot,
            teloxide::types::ChatId(chat_id),
            Some(21),
            ask,
            &obs_old,
            (&ctx.cfg, &ctx.cli, &ctx.mgr),
        )
        .await
        .unwrap();
        assert!(!ctx.cli.sessions.is_ask_active("agy-bg-1").await);
        assert!(ctx.cli.sessions.get_ask_state("agy-fg-2").await.is_none());
    }

    #[test]
    fn test_round7_defect02_and_05_catalog_isolation_and_uppercase_suffix() {
        use crate::messenger::telegram::reply_directives::{
            DirectiveOutcome, parse_model_effort_args_for_provider,
            resolve_directive_for_provider, tokenize_model_command_args,
        };
        use crate::session::data::SessionData;

        assert_eq!(
            resolve_directive_for_provider("codex", None, "@model gpt-oss-120b-medium hello"),
            DirectiveOutcome::IncompatibleModel("gpt-oss-120b-medium".to_string())
        );
        assert_eq!(
            resolve_directive_for_provider("antigravity", None, "@model claude-fable-5 hello"),
            DirectiveOutcome::IncompatibleModel("claude-fable-5".to_string())
        );
        assert_eq!(
            resolve_directive_for_provider("antigravity", None, "@model gpt-6-astra hello"),
            DirectiveOutcome::IncompatibleModel("gpt-6-astra".to_string())
        );
        assert_eq!(
            resolve_directive_for_provider("claude", None, "@model gemini-4-carbon hello"),
            DirectiveOutcome::IncompatibleModel("gemini-4-carbon".to_string())
        );
        assert_eq!(
            resolve_directive_for_provider("claude", None, "@model antigravity-default hello"),
            DirectiveOutcome::IncompatibleModel("antigravity-default".to_string())
        );

        assert!(!SessionData::is_model_compatible("gpt-oss-120b-medium", "codex"));
        assert!(SessionData::is_model_compatible("gpt-oss-120b-medium", "antigravity"));
        assert!(!SessionData::is_model_compatible("gpt-6-astra", "antigravity"));
        assert!(!SessionData::is_model_compatible("gemini-4-carbon", "claude"));

        assert_eq!(
            resolve_directive_for_provider("antigravity", Some("high"), "@model GEMINI-4-CARBON-LOW hello"),
            DirectiveOutcome::Resolved("gemini-4-carbon".to_string(), Some("low".to_string()), "hello")
        );
        assert_eq!(
            parse_model_effort_args_for_provider("claude", "CLAUDE-SONNET-5-LOW", Some("high")),
            ("claude-sonnet-5".to_string(), Some("low".to_string()))
        );
        assert!(tokenize_model_command_args("gemini-4-argon turbo").is_err());
    }

    #[tokio::test]
    async fn test_round7_defect03_04_06_boot_merge_effort_callback_and_heartbeat_provider() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::new(&temp_dir);
        let ctx = init_test_context(&temp_dir);

        // DEFECT-03: concurrent callback mutations preserved across cold boot completion
        let mut pre_boot = crate::session::data::SessionData::new(
            892, "tg".into(), Some(9), "antigravity".into(), "gemini-4-argon".into(),
        );
        let mut concurrent = pre_boot.clone();
        concurrent.apply_provider_switch("claude");
        concurrent.model = "claude-sonnet-5".into();
        concurrent.effort = Some("low".into());
        concurrent.language = Some("ko".into());
        crate::messenger::telegram::session_init::merge_booted_session_id(
            &mut pre_boot,
            Some(concurrent),
            "antigravity",
            "booted-agy-sid",
        );
        assert_eq!(pre_boot.provider, "claude");
        assert_eq!(pre_boot.model, "claude-sonnet-5");
        assert_eq!(pre_boot.effort.as_deref(), Some("low"));
        assert_eq!(pre_boot.language.as_deref(), Some("ko"));
        assert_eq!(pre_boot.get_session_id("antigravity"), "booted-agy-sid");

        // DEFECT-04: stale cross-provider or unsupported model effort callbacks rejected
        use crate::messenger::telegram::reply::validate_standalone_effort_payload;
        assert!(validate_standalone_effort_payload("codex:minimal", "antigravity", "gemini-4-carbon").is_err());
        assert!(validate_standalone_effort_payload("minimal", "antigravity", "gemini-4-carbon").is_err());
        assert!(validate_standalone_effort_payload("max", "antigravity", "gemini-4-carbon").is_err());
        assert!(validate_standalone_effort_payload("xhigh", "antigravity", "gemini-4-carbon").is_err());
        assert!(validate_standalone_effort_payload("claude:low", "antigravity", "gemini-4-carbon").is_err());
        assert_eq!(
            validate_standalone_effort_payload("antigravity:xhigh", "antigravity", "gemini-4-hydrogen"),
            Ok("xhigh".to_string())
        );

        // DEFECT-06: mid-heartbeat provider switch preserves new provider and does not inflate message_count
        let key = SessionKey::telegram(893, Some(15));
        let (mut hb_start, _) = ctx.mgr.resolve_session(&key, "antigravity", "gemini-3.8-flash").await.unwrap();
        hb_start.set_session_id("antigravity", "hb-agy-sid");
        ctx.mgr.preserve_session_identity(&hb_start).await.unwrap();

        let mut switched = hb_start.clone();
        switched.apply_provider_switch("claude");
        switched.set_session_id("claude", "hb-claude-sid");
        ctx.mgr.preserve_session_identity(&switched).await.unwrap();

        let bus = Arc::new(crate::bus::bus::MessageBus::new());
        let hb = crate::heartbeat::scheduler::HeartbeatScheduler::new(ctx.cfg.clone(), ctx.mgr.clone(), ctx.cli.clone(), bus);
        let resp = crate::cli::CliResponse {
            result: "HEARTBEAT_OK".to_string(),
            is_error: false,
            session_id: Some("hb-agy-sid".to_string()),
            returncode: Some(0),
            stderr: String::new(),
        };
        hb.finish_heartbeat_turn(&hb_start, "antigravity", "hb-agy-sid", &resp).await;
        let alert_resp = crate::cli::CliResponse {
            result: "Alert: Disk 95%".to_string(),
            is_error: false,
            session_id: Some("hb-agy-sid".to_string()),
            returncode: Some(0),
            stderr: String::new(),
        };
        hb.finish_heartbeat_turn(&hb_start, "antigravity", "hb-agy-sid", &alert_resp).await;
        let final_sess = ctx.mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(final_sess.provider, "claude");
        assert_eq!(final_sess.get_session_id("antigravity"), "hb-agy-sid");
        assert_eq!(final_sess.get_session_id("claude"), "hb-claude-sid");
        assert_eq!(final_sess.provider_sessions["antigravity"].message_count, 0);
        assert_eq!(final_sess.provider_sessions["claude"].message_count, 0);
    }

    #[tokio::test]
    async fn test_round8_d1_memory_lock_and_busy_ask_pty_guards() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::new(&temp_dir);
        let ctx = init_test_context(&temp_dir);

        let chat_id = 910;
        let msg = make_message_json(chat_id, 100, "/memory");

        // 1. When topic lock is already held by an active turn, try_query_live_memory immediately returns false
        let lock = ctx.mgr.lock_pool.get((chat_id, None));
        let held = lock.lock().await;
        let queried = crate::messenger::telegram::commands_session::try_query_live_memory(
            &ctx.bot, &msg, &ctx.cfg, &ctx.mgr, &ctx.cli,
        ).await;
        assert!(!queried);
        assert!(ctx.mgr.get_active(&SessionKey::telegram(chat_id, None)).await.unwrap().is_none());
        drop(held);

        // 2. When an open ask_question or active turn exists on warm PTY, live memory skips & run_in_pty_session refuses
        let key = SessionKey::telegram(chat_id, None);
        let (mut sess, _) = ctx.mgr.resolve_session(&key, "antigravity", "gemini-4-argon").await.unwrap();
        sess.set_session_id("antigravity", "agy-live-ask");
        ctx.mgr.preserve_session_identity(&sess).await.unwrap();
        let mut env = std::collections::HashMap::new();
        env.insert("TUNER_CHAT_ID".to_string(), chat_id.to_string());
        ctx.cli.sessions.ensure_session("agy-live-ask", temp_dir.path(), "cat", &[], &env).await.unwrap();
        ctx.cli.sessions.set_ask_active("agy-live-ask", true).await;

        assert!(crate::messenger::telegram::handler::is_topic_busy_or_asking(&ctx.mgr, &ctx.cli, chat_id, None).await);
        let queried_during_ask = crate::messenger::telegram::commands_session::try_query_live_memory(
            &ctx.bot, &msg, &ctx.cfg, &ctx.mgr, &ctx.cli,
        ).await;
        assert!(!queried_during_ask);

        let err = ctx.cli.run_in_pty_session("agy-live-ask", "memory prompt", &env, temp_dir.path()).await.unwrap_err();
        assert!(err.starts_with("AGY_ERROR:"));
        assert!(ctx.cli.sessions.is_active("agy-live-ask").await);
        assert!(ctx.cli.sessions.is_ask_active("agy-live-ask").await);
        ctx.cli.terminate_session_clean("agy-live-ask").await;
    }

    #[tokio::test]
    async fn test_round8_d2_dekker_fence_ask_registration_vs_provider_switch() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::new(&temp_dir);
        let ctx = init_test_context(&temp_dir);

        let ask = vec![crate::cli::AskQuestionData {
            question: "Pick?".to_string(),
            options: vec!["A".to_string(), "B".to_string()],
            is_multi_select: false,
        }];

        // Interleaving 1: /provider switches during sendMessage BEFORE set_ask_state runs
        let key1 = SessionKey::telegram(911, Some(3));
        let (mut turn_snap, _) = ctx.mgr.resolve_session(&key1, "claude", "claude-fable-5").await.unwrap();
        turn_snap.set_session_id("claude", "claude-ask-race-1");
        ctx.mgr.preserve_session_identity(&turn_snap).await.unwrap();

        let mut switch_sess = turn_snap.clone();
        crate::messenger::telegram::commands_provider::switch_session_provider(
            "codex", &ctx.cfg, &ctx.mgr, &ctx.cli, &mut switch_sess,
        ).await;
        assert_eq!(switch_sess.provider, "codex");
        assert!(!ctx.cli.sessions.is_ask_active("claude-ask-race-1").await);

        crate::messenger::telegram::commands_provider::register_and_verify_ask_state(
            &ctx.bot, teloxide::types::ChatId(911), 777, ask.clone(), &turn_snap, (&ctx.cfg, &ctx.cli, &ctx.mgr),
        ).await;
        assert!(!ctx.cli.sessions.is_ask_active("claude-ask-race-1").await);

        // Interleaving 2: set_ask_state completes BEFORE /provider switches
        let key2 = SessionKey::telegram(912, Some(4));
        let (mut turn_snap2, _) = ctx.mgr.resolve_session(&key2, "claude", "claude-fable-5").await.unwrap();
        turn_snap2.set_session_id("claude", "claude-ask-race-2");
        ctx.mgr.preserve_session_identity(&turn_snap2).await.unwrap();

        crate::messenger::telegram::commands_provider::register_and_verify_ask_state(
            &ctx.bot, teloxide::types::ChatId(912), 778, ask, &turn_snap2, (&ctx.cfg, &ctx.cli, &ctx.mgr),
        ).await;
        assert!(ctx.cli.sessions.is_ask_active("claude-ask-race-2").await);

        let mut switch_sess2 = turn_snap2.clone();
        crate::messenger::telegram::commands_provider::switch_session_provider(
            "codex", &ctx.cfg, &ctx.mgr, &ctx.cli, &mut switch_sess2,
        ).await;
        assert!(!ctx.cli.sessions.is_ask_active("claude-ask-race-2").await);
    }

    #[tokio::test]
    async fn test_round8_d3_atomic_mutate_and_empty_provider_message_count() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::new(&temp_dir);
        let ctx = init_test_context(&temp_dir);

        let key = SessionKey::telegram(913, Some(5));
        let mut raw = crate::session::data::SessionData::new(913, "tg".into(), Some(5), String::new(), "gemini-4-argon".into());
        raw.set_session_id("antigravity", "agy-empty-prov");
        let saved = ctx.mgr.update_session(&raw).await.unwrap();
        assert!(!saved.provider_sessions.contains_key(""));
        assert_eq!(saved.provider_sessions["antigravity"].message_count, 1);

        // Concurrent callback mutation preserved when persist_stream_session_update runs
        ctx.mgr.mutate_session(&key, None, |cur| {
            cur.apply_provider_switch("claude");
            cur.model = "claude-sonnet-5".into();
            cur.effort = Some("low".into());
            cur.language = Some("ko".into());
            true
        }).await.unwrap();

        let mut updated = raw.clone();
        updated.last_progress_msg_id = Some(42);
        crate::messenger::telegram::stream::persist_stream_session_update(
            &ctx.mgr, &raw, updated, Some("agy-empty-prov".to_string()), &ctx.cfg,
        ).await;

        let after = ctx.mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(after.provider, "claude");
        assert_eq!(after.model, "claude-sonnet-5");
        assert_eq!(after.effort.as_deref(), Some("low"));
        assert_eq!(after.language.as_deref(), Some("ko"));
        assert_eq!(after.last_progress_msg_id, Some(42));
        assert!(!after.provider_sessions.contains_key(""));
        assert_eq!(after.provider_sessions["antigravity"].message_count, 2);
    }

    #[tokio::test]
    async fn test_round8_d4_cross_topic_new_lock_and_reset_topic_sessions() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::new(&temp_dir);
        let ctx = init_test_context(&temp_dir);

        // 1. reset_topic_sessions preserves live callback fields while clearing all provider_sessions
        let key = SessionKey::telegram(914, Some(77));
        let (mut s, _) = ctx.mgr.resolve_session(&key, "antigravity", "gemini-4-argon").await.unwrap();
        s.set_session_id("antigravity", "old-agy");
        s.set_session_id("claude", "old-claude");
        s.set_session_id("codex", "old-codex");
        s.apply_provider_switch("claude");
        s.model = "claude-sonnet-5".into();
        s.effort = Some("max".into());
        s.language = Some("ko".into());
        ctx.mgr.preserve_session_identity(&s).await.unwrap();

        let reset = ctx.mgr.reset_topic_sessions(&key, "antigravity", "antigravity-default", Some("antigravity"), Some("Marketing".into())).await.unwrap();
        assert_eq!(reset.provider, "claude");
        assert_eq!(reset.model, "claude-sonnet-5");
        assert_eq!(reset.effort.as_deref(), Some("max"));
        assert_eq!(reset.language.as_deref(), Some("ko"));
        assert_eq!(reset.topic_name.as_deref(), Some("Marketing"));
        assert_eq!(reset.get_session_id("antigravity"), "");
        assert_eq!(reset.get_session_id("claude"), "");
        assert_eq!(reset.get_session_id("codex"), "old-codex");
        assert!(reset.get_past_session_ids("antigravity").contains(&"old-agy".to_string()));
        assert!(reset.get_past_session_ids("claude").contains(&"old-claude".to_string()));

        // 2. Cross-topic `/new Marketing` from General blocks behind target topic (914, Some(77)) lock
        let mut group_cfg = (*ctx.cfg).clone();
        group_cfg.allowed_group_ids = vec![914];
        let group_cfg = Arc::new(group_cfg);
        ctx.topic_cache.insert(914, 77, "Marketing".to_string());
        let target_lock = ctx.mgr.lock_pool.get((914, Some(77)));
        let target_guard = target_lock.lock().await;

        let general_new_msg = make_msg(
            r#"{"message_id":201,"date":1,"chat":{"id":914,"type":"supergroup","is_forum":true},"from":{"id":100,"is_bot":false,"first_name":"U"},"text":"/new Marketing"}"#,
        );
        let mgm = Arc::new(crate::telegram::media_group::MediaGroupManager::new());
        let blocked = tokio::time::timeout(
            std::time::Duration::from_millis(80),
            handle_message(
                ctx.bot.clone(),
                general_new_msg,
                group_cfg,
                ctx.mgr.clone(),
                ctx.cli.clone(),
                (ctx.cron_mgr.clone(), ctx.topic_cache.clone()),
                (ctx.bot_info.clone(), mgm),
            ),
        ).await;
        assert!(blocked.is_err(), "cross-topic /new must wait on target_topic lock");
        drop(target_guard);
    }
}
