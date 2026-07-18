use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::antigravity::AntigravityCli;
use crate::session::manager::SessionManager;
use std::sync::Arc;
use super::topic_cache::{BotInfo, TopicNameCache};
use super::commands;
use super::reply;
use super::callbacks;
use super::handle_message;

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
    bus.register_transport(Arc::new(super::transport::TelegramTransport::new(bot.clone())));
    Arc::new(crate::heartbeat::scheduler::HeartbeatScheduler::new(cfg.clone(), sess, cli.clone(), bus.clone())).start();
    let cron = Arc::new(crate::cron::manager::CronManager::new(std::path::PathBuf::from(home).join(".tuner/cron_jobs.json")));
    Arc::new(crate::cron::scheduler::CronScheduler::new(cfg.clone(), cron.clone(), cli, bus)).start();
    let clean = Arc::new(crate::cleanup::observer::CleanupObserver::new(cfg.cleanup.clone(), cfg.working_dir.join("telegram_files"), cfg.working_dir.join("output_to_user")));
    tokio::spawn(async move { clean.start().await; });
    cron
}

pub async fn run_bot(config: CliConfig) -> Result<(), String> {
    let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or(config.telegram_token.clone());
    if tok.is_empty() { return Err("No token".to_string()); }
    let bot = Bot::new(tok);
    let _ = commands::register_commands(&bot).await;
    let bot_info = Arc::new(BotInfo { username: bot.get_me().await.ok().and_then(|m| m.user.username) });
    let config_arc = Arc::new(config);
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    
    reply::spawn_restart_watcher(home.clone());
    let topic_cache = Arc::new(TopicNameCache::new());
    let p = std::path::Path::new(&home).join(".tuner/sessions.json");
    let sessions = Arc::new(build_sessions(p, topic_cache.clone()));

    reply::load_sessions_cache(&sessions, &topic_cache).await;
    let cli = Arc::new(AntigravityCli::new((*config_arc).clone()));
    let cron_manager = start_schedulers(bot.clone(), config_arc.clone(), sessions.clone(), cli.clone(), &home);

    let (b, s) = (bot.clone(), sessions.clone());
    tokio::spawn(async move { reply::send_startup_notification(b, s).await; });

    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(callbacks::handle_callback_query));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![config_arc, sessions, cli, cron_manager, topic_cache, bot_info])
        .build().dispatch().await;
    Ok(())
}
