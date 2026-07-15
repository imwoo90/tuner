use crate::telegram::handle_message;
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::session::key::SessionKey;
use crate::session::manager::SessionManager;
use std::sync::Arc;
use teloxide::Bot;
use teloxide::types::Message;
use crate::telegram::{TopicNameCache, BotInfo};

fn setup() -> (
    Arc<SessionManager>,
    Arc<CliConfig>,
    Arc<AntigravityCli>,
    Bot,
    Arc<crate::cron::manager::CronManager>,
    Arc<TopicNameCache>,
    Arc<BotInfo>
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
    let cron_mgr = Arc::new(crate::cron::manager::CronManager::new(temp_cron.path().to_path_buf()));
    let topic_cache = Arc::new(TopicNameCache::new());
    let bot_info = Arc::new(BotInfo { username: Some("my_bot".to_string()) });
    (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info)
}

fn make_msg(json: &str) -> Message {
    serde_json::from_str(json).unwrap()
}

#[tokio::test]
async fn test_telegram_command_new() {
    let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info) = setup();
    let key = SessionKey::telegram(123, None);
    let (sess, _) = mgr.resolve_session(&key, &cfg.provider, "opus").await.unwrap();
    let mut updated = sess.clone();
    updated.set_session_id("antigravity", "active-conv-xyz");
    mgr.update_session(&updated, 0.0, 0).await.unwrap();

    let msg = make_msg(r#"{"message_id":3,"date":1,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I","username":"u"},"text":"/new"}"#);
    handle_message(bot, msg, cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info).await.unwrap();

    let s_after = mgr.get_active(&key).await.unwrap().unwrap();
    assert_eq!(s_after.get_session_id("antigravity"), "");
}

#[tokio::test]
async fn test_telegram_command_abort() {
    let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info) = setup();
    let msg = make_msg(r#"{"message_id":4,"date":1,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I","username":"u"},"text":"/abort"}"#);
    let res = handle_message(bot, msg, cfg, mgr, cli, cron_mgr, topic_cache, bot_info).await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn test_telegram_command_model() {
    let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info) = setup();
    let key = SessionKey::telegram(123, None);
    let (sess, _) = mgr.resolve_session(&key, &cfg.provider, "opus").await.unwrap();
    assert_eq!(sess.model, "opus");

    let msg = make_msg(r#"{"message_id":5,"date":1,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I","username":"u"},"text":"/model sonnet"}"#);
    handle_message(bot.clone(), msg, cfg.clone(), mgr.clone(), cli.clone(), cron_mgr.clone(), topic_cache.clone(), bot_info.clone()).await.unwrap();

    let s_after = mgr.get_active(&key).await.unwrap().unwrap();
    assert_eq!(s_after.model, "sonnet");

    let msg_interactive = make_msg(r#"{"message_id":6,"date":1,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I","username":"u"},"text":"/model"}"#);
    let res = handle_message(bot, msg_interactive, cfg, mgr, cli, cron_mgr, topic_cache, bot_info).await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn test_telegram_command_diagnose() {
    let (mgr, cfg, cli, bot, _, topic_cache, _) = setup();
    let cron_mgr = crate::cron::manager::CronManager::new(tempfile::NamedTempFile::new().unwrap().path().to_path_buf());
    let msg = make_msg(r#"{"message_id":7,"date":1,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I","username":"u"},"text":"/diagnose"}"#);
    let res = crate::telegram::commands::handle_commands(&bot, &msg, "/diagnose", &cfg, &mgr, &cli, &cron_mgr, &topic_cache).await;
    assert!(res.is_ok());
    assert_eq!(res.unwrap(), true);
}

#[tokio::test]
async fn test_telegram_command_memory() {
    let (mgr, cfg, cli, bot, _, topic_cache, _) = setup();
    let cron_mgr = crate::cron::manager::CronManager::new(tempfile::NamedTempFile::new().unwrap().path().to_path_buf());
    let msg = make_msg(r#"{"message_id":8,"date":1,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I","username":"u"},"text":"/memory"}"#);
    let res = crate::telegram::commands::handle_commands(&bot, &msg, "/memory", &cfg, &mgr, &cli, &cron_mgr, &topic_cache).await;
    assert!(res.is_ok());
    assert_eq!(res.unwrap(), true);
}

#[tokio::test]
async fn test_telegram_command_stop_scoped() {
    let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info) = setup();
    let mut env = std::collections::HashMap::new();
    env.insert("TUNER_CHAT_ID".to_string(), "123".to_string());
    env.insert("TUNER_TOPIC_ID".to_string(), "456".to_string());
    cli.sessions.ensure_session("sess-stop-test", &std::path::PathBuf::from("."), "cat", &[], &env).await.unwrap();
    assert!(cli.sessions.is_active("sess-stop-test").await);

    let msg = make_msg(r#"{"message_id":9,"date":1,"chat":{"id":123,"type":"supergroup","is_forum":true},"from":{"id":100,"is_bot":false,"first_name":"I"},"text":"/stop","message_thread_id":456,"is_topic_message":true}"#);
    handle_message(bot, msg, cfg, mgr, cli.clone(), cron_mgr, topic_cache, bot_info).await.unwrap();

    assert!(!cli.sessions.is_active("sess-stop-test").await);
}

#[tokio::test]
async fn test_telegram_command_stop_all() {
    let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info) = setup();
    let mut env = std::collections::HashMap::new();
    env.insert("TUNER_CHAT_ID".to_string(), "123".to_string());
    cli.sessions.ensure_session("sess-stop-all-test", &std::path::PathBuf::from("."), "cat", &[], &env).await.unwrap();
    assert!(cli.sessions.is_active("sess-stop-all-test").await);

    let msg = make_msg(r#"{"message_id":10,"date":1,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"text":"/stop_all"}"#);
    handle_message(bot, msg, cfg, mgr, cli.clone(), cron_mgr, topic_cache, bot_info).await.unwrap();

    assert!(!cli.sessions.is_active("sess-stop-all-test").await);
}

#[tokio::test]
async fn test_telegram_command_reset_alias() {
    let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info) = setup();
    let key = SessionKey::telegram(123, None);
    let (sess, _) = mgr.resolve_session(&key, &cfg.provider, "opus").await.unwrap();
    let mut updated = sess.clone();
    updated.set_session_id("antigravity", "active-conv-reset");
    mgr.update_session(&updated, 0.0, 0).await.unwrap();

    let msg = make_msg(r#"{"message_id":11,"date":1,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"text":"/reset"}"#);
    handle_message(bot, msg, cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info).await.unwrap();

    let s_after = mgr.get_active(&key).await.unwrap().unwrap();
    assert_eq!(s_after.get_session_id("antigravity"), "");
}

#[tokio::test]
async fn test_telegram_command_new_with_topicname() {
    let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info) = setup();
    let key = SessionKey::telegram(123, Some(777));
    
    // Seed named topic "Deployments" inside cache
    topic_cache.insert(123, 777, "Deployments".to_string());

    let (sess, _) = mgr.resolve_session(&key, &cfg.provider, "opus").await.unwrap();
    let mut updated = sess.clone();
    updated.set_session_id("antigravity", "topic-777-session");
    mgr.update_session(&updated, 0.0, 0).await.unwrap();

    let msg = make_msg(r#"{"message_id":12,"date":1,"chat":{"id":123,"type":"supergroup","is_forum":true},"from":{"id":100,"is_bot":false,"first_name":"I"},"text":"/new @Deployments"}"#);
    handle_message(bot, msg, cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info).await.unwrap();

    let s_after = mgr.get_active(&key).await.unwrap().unwrap();
    assert_eq!(s_after.get_session_id("antigravity"), "");
}

#[tokio::test]
async fn test_telegram_forum_topic_events() {
    let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info) = setup();
    let msg_created = make_msg(r#"{"message_id":13,"date":1,"chat":{"id":123,"type":"supergroup"},"from":{"id":100,"is_bot":false},"forum_topic_created":{"name":"QA Thread","icon_color":0},"message_thread_id":999}"#);
    handle_message(bot.clone(), msg_created, cfg.clone(), mgr.clone(), cli.clone(), cron_mgr.clone(), topic_cache.clone(), bot_info.clone()).await.unwrap();

    // Verify it is inserted in topic_cache
    assert_eq!(topic_cache.find_by_name(123, "@QA Thread"), Some(999));

    let msg_edited = make_msg(r#"{"message_id":14,"date":1,"chat":{"id":123,"type":"supergroup"},"from":{"id":100,"is_bot":false},"forum_topic_edited":{"name":"QA & Testing Thread"},"message_thread_id":999}"#);
    handle_message(bot.clone(), msg_edited, cfg.clone(), mgr.clone(), cli.clone(), cron_mgr.clone(), topic_cache.clone(), bot_info.clone()).await.unwrap();

    // Verify it is updated in topic_cache
    assert_eq!(topic_cache.find_by_name(123, "@QA & Testing Thread"), Some(999));
}

#[tokio::test]
async fn test_telegram_commands_specification() {
    let commands = crate::telegram::commands::get_bot_commands();
    assert!(!commands.is_empty());
    for cmd in &commands {
        assert!(cmd.command.len() >= 1 && cmd.command.len() <= 32);
        for c in cmd.command.chars() {
            assert!(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        }
        assert!(cmd.description.len() >= 1 && cmd.description.len() <= 256);
    }
    let names: Vec<&str> = commands.iter().map(|c| c.command.as_str()).collect();
    for n in &["help", "new", "reset", "stop", "model", "plan", "grill_me", "goal", "learn", "teamwork_preview"] {
        assert!(names.contains(n));
    }
}
