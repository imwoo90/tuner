//! # Telegram Bot Event Loop Runner
//!
//! ## Overview
//! Connects to the Telegram Bot API and starts long-polling listener using Teloxide. Coordinates
//! session caches, message queues, and worker status.
//!
//! ## Search Tags
//! #polling-loop, #bot-initializer, #teloxide-runner

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
use crate::workspace::paths::DuctorPaths;

pub(crate) fn build_sessions(path: std::path::PathBuf, cache: Arc<TopicNameCache>, config: &CliConfig) -> SessionManager {
    let tz = config.user_timezone.clone().unwrap_or_else(|| "UTC".to_string());
    SessionManager::new(
        path,
        config.idle_timeout_minutes,
        config.daily_reset_hour,
        config.daily_reset_enabled,
        tz,
        config.max_session_messages,
    )
    .with_topic_resolver(Arc::new(move |c, t| cache.find_by_id(c, t)))
}

fn start_schedulers(
    bot: Bot,
    cfg: Arc<CliConfig>,
    sess: Arc<SessionManager>,
    cli: Arc<AntigravityCli>,
    paths: DuctorPaths,
) -> Arc<crate::cron::manager::CronManager> {
    let bus = Arc::new(crate::bus::bus::MessageBus::new());
    bus.register_transport(Arc::new(super::transport::TelegramTransport::new(bot.clone())));
    bus.set_injector(Arc::new(super::transport::TelegramPromptInjector::new(
        cli.clone(),
        sess.clone(),
        cfg.clone(),
    )));
    Arc::new(crate::heartbeat::scheduler::HeartbeatScheduler::new(cfg.clone(), sess, cli.clone(), bus.clone())).start();
    let cron = Arc::new(crate::cron::manager::CronManager::new(paths.cron_jobs_path()));
    Arc::new(crate::cron::scheduler::CronScheduler::new(cfg.clone(), cron.clone(), cli.clone(), bus.clone())).start();
    let clean = Arc::new(crate::cleanup::observer::CleanupObserver::new(cfg.cleanup.clone(), cfg.working_dir.join("telegram_files"), cfg.working_dir.join("output_to_user")));
    tokio::spawn(async move { clean.start().await; });

    if cfg.webhooks.enabled {
        let wh_mgr = Arc::new(crate::webhook::manager::WebhookManager::new(paths.webhooks_path()));
        let wh_obs = Arc::new(crate::webhook::observer::WebhookObserver::new(
            wh_mgr,
            paths.webhooks_path(),
            cfg.clone(),
            cli,
        ));
        let obs_wire = crate::bus::observers_wire::ObserverManager {
            heartbeat: None,
            cron: None,
            background: None,
            webhook: Some(wh_obs.clone()),
        };
        tokio::spawn(async move {
            obs_wire.wire_to_bus(bus, None).await;
            if let Err(e) = wh_obs.start().await {
                eprintln!("❌ [tuner] Failed to start Webhook server: {}", e);
            }
        });
    }

    cron
}

async fn init_telegram_bot(tok: String, profile: Option<String>) -> (Bot, Arc<BotInfo>) {
    if tok.is_empty() || tok == "YOUR_BOT_TOKEN_HERE" || tok.starts_with("YOUR_") {
        eprintln!("❌ [tuner] Profile '{}' Telegram token is not configured or is placeholder. Worker will sleep to prevent tight restart loop.", profile.as_deref().unwrap_or("default"));
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
        }
    }
    let bot = Bot::new(tok);
    match bot.get_me().await {
        Ok(me) => {
            let bot_info = Arc::new(BotInfo { username: me.user.username });
            (bot, bot_info)
        }
        Err(e) => {
            eprintln!("❌ [tuner] Profile '{}' failed to validate Telegram token: {}. Worker will sleep to prevent tight restart loop.", profile.as_deref().unwrap_or("default"), e);
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            }
        }
    }
}

fn spawn_restart_watcher(tuner_home: std::path::PathBuf) {
    tokio::spawn(async move {
        let marker = tuner_home.join("restart-requested");
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
        loop {
            interval.tick().await;
            if marker.exists() {
                let _ = std::fs::remove_file(&marker);
                println!("🤖 [tuner] Restart requested via marker. Exiting...");
                std::process::exit(42);
            }
        }
    });
}

fn spawn_web_warmup(paths: &DuctorPaths) {
    let review_path = paths.review_sessions_path();
    let vault_prof = paths.profile.clone().unwrap_or_else(|| "default".to_string());
    let vault_dir = paths.vault_dir();
    tokio::spawn(async move {
        super::vault::global_vault_manager()
            .init_storage(vault_prof, vault_dir)
            .await;
        let mgr = super::review::global_review_manager();
        mgr.init_storage(review_path).await;
        mgr.warmup().await;
    });
}

/// Executes `run_bot` (run bot).
pub async fn run_bot(config: CliConfig, paths: DuctorPaths) -> Result<(), String> {
    let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or(config.telegram_token.clone());
    let (bot, bot_info) = init_telegram_bot(tok, paths.profile.clone()).await;
    let _ = commands::register_commands(&bot).await;
    let config_arc = Arc::new(config);
    
    spawn_restart_watcher(paths.tuner_home.clone());
    let topic_cache = Arc::new(TopicNameCache::new());
    let p = paths.sessions_path();
    let sessions = Arc::new(build_sessions(p, topic_cache.clone(), &config_arc));

    reply::load_sessions_cache(&sessions, &topic_cache).await;
    let cli = Arc::new(AntigravityCli::new((*config_arc).clone()));
    let cron_manager = start_schedulers(bot.clone(), config_arc.clone(), sessions.clone(), cli.clone(), paths.clone());
    let media_group_manager = Arc::new(super::media_group::MediaGroupManager::new());

    let (b, s) = (bot.clone(), sessions.clone());
    tokio::spawn(async move { reply::send_startup_notification(b, s).await; });
    spawn_web_warmup(&paths);

    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(callbacks::handle_callback_query));

    let aux_deps = (cron_manager.clone(), topic_cache.clone());
    let meta_deps = (bot_info.clone(), media_group_manager.clone());
    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![config_arc, sessions, cli, cron_manager, topic_cache, bot_info, media_group_manager, aux_deps, meta_deps])
        .build().dispatch().await;
    Ok(())
}
