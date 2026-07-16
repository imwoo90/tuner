
use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::cli::AgentProvider;
use crate::session::manager::SessionManager;
use crate::session::data::SessionData;
use std::sync::Arc;

pub mod formatting;
#[cfg(test)]
pub mod formatting_tests;
pub mod reply;
#[cfg(test)]
pub mod reply_tests;
#[cfg(test)]
pub mod handler_tests;
pub mod commands;
pub mod cron_selector;
pub mod topic_cache;
pub mod stream;
pub mod transport;
pub mod lang;
pub mod callbacks;

pub(crate) use reply::{build_reply_prompt, parse_model_directive};
pub use transport::TelegramTransport;
pub mod typing;


pub(crate) fn get_topic_id(msg: &Message) -> Option<i64> {
    match &msg.kind {
        teloxide::types::MessageKind::Common(c) if c.is_topic_message => msg.thread_id.map(|t| t as i64),
        _ => None,
    }
}

pub use topic_cache::{BotInfo, TopicNameCache};

async fn run_cli_stream(
    bot: &Bot,
    msg: &Message,
    prompt: &str,
    sid: &str,
    cli: &AntigravityCli,
    sessions: &SessionManager,
    sess: SessionData,
    config: &CliConfig,
) -> Result<(), teloxide::RequestError> {
    let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
    let _guard = typing::TelegramTypingGuard::new(bot.clone(), tok, msg).await;
    let opt_sid = (!sid.is_empty()).then_some(sid);
    let mut cli_clone = (*cli).clone();
    if !sess.model.is_empty() {
        cli_clone.config.model = Some(sess.model.clone());
    }
    let stream_res = cli_clone.send_streaming(prompt, opt_sid, false, config.working_dir.clone()).await;
    match stream_res {
        Ok(s) => {
            stream::consume_stream(bot, msg.chat.id, msg.thread_id, s, sessions, sess, config).await?;
        }
        Err(e) => {
            eprintln!("CLI ERROR: {:?}", e);
            let mut req = bot.send_message(msg.chat.id, format!("❌ Error: {}", e));
            if let Some(t) = msg.thread_id { req = req.message_thread_id(t); }
            let _ = req.await;
        }
    }
    Ok(())
}

async fn process_text(
    bot: &Bot,
    msg: &Message,
    text: &str,
    config: &CliConfig,
    sessions: &SessionManager,
    cli: &AntigravityCli,
    cron_manager: &crate::cron::manager::CronManager,
    topic_cache: &TopicNameCache,
) -> Result<(), teloxide::RequestError> {
    if commands::handle_commands(bot, msg, text, config, sessions, cli, cron_manager, topic_cache).await? {
        return Ok(());
    }

    let (model_override, current_text) = parse_model_directive(text);
    let topic_id = get_topic_id(msg);
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, topic_id);
    let mut model = config.model.as_deref().unwrap_or("antigravity-default").to_string();
    if let Some(ref m) = model_override { model = m.clone(); }

    let (mut sess, _) = sessions.resolve_session(&key, &config.provider, &model).await.unwrap();
    if let Some(ref m) = model_override {
        sess.model = m.clone();
        let _ = sessions.update_session(&sess, 0.0, 0).await;
        if current_text.is_empty() {
            let mut req = bot.send_message(msg.chat.id, format!("Next message will use {}", m));
            if let Some(t) = msg.thread_id { req = req.message_thread_id(t); }
            let _ = req.await;
            return Ok(());
        }
    }

    let mut prompt = build_reply_prompt(msg, current_text);
    let _ = reply::download_and_inject_media_hint(bot, msg, &config.working_dir, &mut prompt).await;

    let session_id = sess.get_session_id(&config.provider);
    if cli.sessions.is_active(&session_id).await && cli.sessions.is_running(&session_id).await {
        let input_prompt = format!("{}\r", current_text);
        println!("🤖 [tuner] Feed active session {}: {:?}", session_id, input_prompt);
        let _ = cli.sessions.write_to_session(&session_id, &input_prompt).await;
        return Ok(());
    }

    run_cli_stream(bot, msg, &prompt, &session_id, cli, sessions, sess, config).await
}

async fn handle_message_inner(
    bot: Bot,
    msg: Message,
    config: Arc<CliConfig>,
    sessions: Arc<SessionManager>,
    cli: Arc<AntigravityCli>,
    cron_manager: Arc<crate::cron::manager::CronManager>,
    topic_cache: Arc<TopicNameCache>,
    bot_info: Arc<BotInfo>,
) -> Result<(), teloxide::RequestError> {
    let from_id = msg.from().map(|u| u.id.0 as i64).unwrap_or(0);
    let chat_id_raw = msg.chat.id.0;
    eprintln!("🤖 [tuner] Incoming message: from_id={}, chat_id={}", from_id, chat_id_raw);

    if let Some(target_chat_id) = msg.migrate_to_chat_id() {
        let _ = sessions.migrate_chat_id(chat_id_raw, target_chat_id.0).await;
        return Ok(());
    }

    if topic_cache::handle_forum_topic_events(&msg, &topic_cache, chat_id_raw) {
        return Ok(());
    }

    let ok = if msg.chat.is_group() || msg.chat.is_supergroup() {
        config.allowed_group_ids.contains(&chat_id_raw) && config.allowed_user_ids.contains(&from_id)
    } else {
        config.allowed_user_ids.contains(&from_id)
    };
    if ok {
        let has_med = reply::has_media(&msg);
        let mut text = reply::strip_mention(msg.text().or(msg.caption()).unwrap_or(""), bot_info.username.as_deref());
        if text.starts_with("/teamwork_preview") {
            text = text.replacen("/teamwork_preview", "/teamwork-preview", 1);
        } else if text.starts_with("/grill_me") {
            text = text.replacen("/grill_me", "/grill-me", 1);
        }
        if !text.is_empty() || has_med {
            process_text(&bot, &msg, &text, &config, &sessions, &cli, &cron_manager, &topic_cache).await?;
        }
    }
    Ok(())
}

pub(crate) async fn handle_message(
    bot: Bot,
    msg: Message,
    config: Arc<CliConfig>,
    sessions: Arc<SessionManager>,
    cli: Arc<AntigravityCli>,
    cron_manager: Arc<crate::cron::manager::CronManager>,
    topic_cache: Arc<TopicNameCache>,
    bot_info: Arc<BotInfo>,
) -> Result<(), teloxide::RequestError> {
    let topic_id = get_topic_id(&msg);
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, topic_id);
    let default_model = config.model.as_deref().unwrap_or("antigravity-default");
    
    let active_lang = if let Ok((sess, _)) = sessions.resolve_session(&key, &config.provider, default_model).await {
        sess.language.unwrap_or_else(|| config.language.clone().unwrap_or_else(|| "en".to_string()))
    } else {
        config.language.clone().unwrap_or_else(|| "en".to_string())
    };

    if cfg!(test) {
        crate::i18n::TASK_ACTIVE_LANG.scope(active_lang, async move {
            handle_message_inner(bot, msg, config, sessions, cli, cron_manager, topic_cache, bot_info).await
        }).await
    } else {
        tokio::spawn(async move {
            let _ = crate::i18n::TASK_ACTIVE_LANG.scope(active_lang, async move {
                handle_message_inner(bot, msg, config, sessions, cli, cron_manager, topic_cache, bot_info).await
            }).await;
        });
        Ok(())
    }
}

fn build_sessions(path: std::path::PathBuf, cache: Arc<TopicNameCache>) -> SessionManager {
    SessionManager::new(path, 30, 4, false, "UTC".to_string(), None)
        .with_topic_resolver(Arc::new(move |c, t| cache.find_by_id(c, t)))
}
fn start_schedulers(
    bot: Bot,
    cfg: Arc<CliConfig>,
    sess: Arc<SessionManager>,
    cli: Arc<AntigravityCli>,
    home: &str,
) -> Arc<crate::cron::manager::CronManager> {
    let bus = Arc::new(crate::bus::bus::MessageBus::new());
    bus.register_transport(Arc::new(TelegramTransport::new(bot.clone())));
    Arc::new(crate::heartbeat::scheduler::HeartbeatScheduler::new(cfg.clone(), sess, cli.clone(), bus.clone())).start();
    let cron = Arc::new(crate::cron::manager::CronManager::new(std::path::PathBuf::from(home).join(".tuner/cron_jobs.json")));
    Arc::new(crate::cron::scheduler::CronScheduler::new(cfg.clone(), cron.clone(), cli, bus)).start();
    let clean = Arc::new(crate::cleanup::observer::CleanupObserver::new(cfg.cleanup.clone(), cfg.working_dir.join("telegram_files"), cfg.working_dir.join("output_to_user")));
    tokio::spawn(async move { clean.start().await; });
    cron
}

pub async fn run_bot(config: CliConfig) -> Result<(), String> {
    let token = std::env::var("TELEGRAM_TOKEN").unwrap_or(config.telegram_token.clone());
    if token.is_empty() { return Err("No token".to_string()); }
    let bot = Bot::new(token);
    let _ = commands::register_commands(&bot).await;
    let bot_info = Arc::new(BotInfo { username: bot.get_me().await.ok().and_then(|m| m.user.username) });
    let config_arc = Arc::new(config);
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/wimvm".to_string());
    
    reply::spawn_restart_watcher(home.clone());
    
    let topic_cache = Arc::new(TopicNameCache::new());
    let sessions = Arc::new(build_sessions(std::path::PathBuf::from(&home).join(".tuner/sessions.json"), topic_cache.clone()));

    reply::load_sessions_cache(&sessions, &topic_cache).await;
    
    let cli = Arc::new(AntigravityCli::new((*config_arc).clone()));
    let cron_manager = start_schedulers(bot.clone(), config_arc.clone(), sessions.clone(), cli.clone(), &home);

    let bot_clone = bot.clone();
    let sessions_clone = sessions.clone();
    tokio::spawn(async move {
        reply::send_startup_notification(bot_clone, sessions_clone).await;
    });

    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(callbacks::handle_callback_query));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![config_arc, sessions, cli, cron_manager, topic_cache, bot_info])
        .build().dispatch().await;
    Ok(())
}
