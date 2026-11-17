//! # Control-Plane & Messenger Event Loop Runner
//!
//! ## Overview
//! Initializes shared sessions, message bus, cron/heartbeat schedulers, and the embedded
//! `:8743` Web Dashboard before attempting Telegram bot validation, supporting both Co-Resident
//! Telegram+Web mode and Web-Standalone mode when `TELEGRAM_TOKEN` is missing or invalid.
//!
//! ## Search Tags
//! #polling-loop, #bot-initializer, #teloxide-runner, #web-standalone

use super::topic_cache::{BotInfo, TopicNameCache};
use super::{callbacks, commands, handle_message, reply};
use crate::bus::bus::MessageBus;
use crate::cli::antigravity::AntigravityCli;
use crate::config::CliConfig;
use crate::cron::manager::CronManager;
use crate::messenger::web::{
    auth::WebAuthStore, events::WebEventHub, init_global_web_ctx, WebAppCtx, WebTransport,
};
use crate::session::manager::SessionManager;
use crate::workspace::paths::TunerPaths;
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::RwLock;

pub(crate) fn build_sessions(
    path: std::path::PathBuf,
    cache: Arc<TopicNameCache>,
    config: &CliConfig,
) -> SessionManager {
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

struct CoreRuntime {
    cfg: Arc<CliConfig>,
    topic_cache: Arc<TopicNameCache>,
    sessions: Arc<SessionManager>,
    cli: Arc<AntigravityCli>,
    bus: Arc<MessageBus>,
    cron: Arc<CronManager>,
    web_ctx: Arc<WebAppCtx>,
}

async fn init_core_runtime(config: CliConfig, paths: &TunerPaths) -> CoreRuntime {
    let cfg = Arc::new(config);
    spawn_restart_watcher(paths.tuner_home.clone());
    let topic_cache = Arc::new(TopicNameCache::new());
    let sessions = Arc::new(build_sessions(paths.sessions_path(), topic_cache.clone(), &cfg));
    reply::load_sessions_cache(&sessions, &topic_cache).await;
    let cli = Arc::new(AntigravityCli::new((*cfg).clone()));
    let bus = Arc::new(MessageBus::with_lock_pool(sessions.lock_pool.clone()));
    let cron = Arc::new(CronManager::new(paths.cron_jobs_path()));
    let vault_dir = paths.vault_dir();
    let profile = paths.profile.clone().unwrap_or_else(|| "default".to_string());
    let web_ctx = Arc::new(WebAppCtx {
        profile,
        tuner_home: paths.tuner_home.clone(),
        vault_dir: vault_dir.clone(),
        working_dir: paths.workspace().clone(),
        config: cfg.clone(),
        sessions: sessions.clone(),
        cli: cli.clone(),
        cron_manager: cron.clone(),
        topic_cache: topic_cache.clone(),
        bus: bus.clone(),
        bot: RwLock::new(None),
        events: Arc::new(WebEventHub::new()),
        auth: Arc::new(WebAuthStore::new(vault_dir)),
    });
    init_global_web_ctx(web_ctx.clone());
    bus.register_transport(Arc::new(WebTransport::new(web_ctx.clone())));
    CoreRuntime { cfg, topic_cache, sessions, cli, bus, cron, web_ctx }
}

fn start_background_schedulers(rt: &CoreRuntime, paths: &TunerPaths) {
    Arc::new(crate::heartbeat::scheduler::HeartbeatScheduler::new(
        rt.cfg.clone(),
        rt.sessions.clone(),
        rt.cli.clone(),
        rt.bus.clone(),
    ))
    .start();
    crate::upgrade::spawn_auto_upgrade_scheduler(rt.cfg.clone(), rt.cli.clone());
    Arc::new(crate::cron::scheduler::CronScheduler::new(
        rt.cfg.clone(),
        rt.cron.clone(),
        rt.bus.clone(),
    ))
    .start();
    let clean = Arc::new(crate::cleanup::observer::CleanupObserver::new(
        rt.cfg.cleanup.clone(),
        paths.telegram_files_dir(),
        paths.output_to_user_dir(),
    ));
    tokio::spawn(async move { clean.start().await; });
    let skill_paths = paths.clone();
    tokio::spawn(async move {
        let _ = crate::workspace::skills::watch_skill_sync(&skill_paths, 30_000).await;
    });
    tokio::spawn(async {
        crate::cli::antigravity::model_spec::ensure_live_catalog_fresh().await;
    });
    spawn_web_warmup(paths);
}

async fn try_init_telegram_bot(tok: String, profile: Option<String>) -> Option<(Bot, Arc<BotInfo>)> {
    let prof_name = profile.as_deref().unwrap_or("default");
    if tok.trim().is_empty() || tok == "YOUR_BOT_TOKEN_HERE" || tok.starts_with("YOUR_") {
        eprintln!("ℹ️ [tuner] Profile '{prof_name}' has no Telegram token; starting Web-Standalone mode.");
        return None;
    }
    let bot = Bot::new(tok);
    match tokio::time::timeout(tokio::time::Duration::from_secs(15), bot.get_me()).await {
        Ok(Ok(me)) => Some((bot, Arc::new(BotInfo { username: me.user.username }))),
        Ok(Err(e)) => {
            eprintln!("⚠️ [tuner] Profile '{prof_name}' failed Telegram token validation ({e}); starting Web-Standalone mode.");
            None
        }
        Err(_) => {
            eprintln!("⚠️ [tuner] Profile '{prof_name}' Telegram API probe timed out; starting Web-Standalone mode.");
            None
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

fn spawn_web_warmup(paths: &TunerPaths) {
    let review_path = paths.review_sessions_path();
    let vault_prof = paths.profile.clone().unwrap_or_else(|| "default".to_string());
    let vault_dir = paths.vault_dir();
    tokio::spawn(async move {
        super::vault::global_vault_manager()
            .init_storage(vault_prof, vault_dir.clone())
            .await;
        let mgr = super::review::global_review_manager();
        mgr.init_storage(review_path).await;
        let port = mgr.ensure_server_running().await;
        crate::security::vault_auth::proxy::publish_tunnel_endpoint(&vault_dir, port).await;
        mgr.warmup().await;
    });
}

async fn run_telegram_dispatcher(rt: CoreRuntime, bot: Bot, bot_info: Arc<BotInfo>) {
    rt.web_ctx.set_bot(bot.clone()).await;
    let targets = if !rt.cfg.allowed_group_ids.is_empty() {
        rt.cfg.allowed_group_ids.clone()
    } else {
        rt.cfg.allowed_user_ids.clone()
    };
    rt.bus.register_transport(Arc::new(
        super::transport::TelegramTransport::with_targets(bot.clone(), targets)
            .with_context(rt.cfg.clone(), rt.sessions.clone()),
    ));
    let _ = commands::register_commands(&bot).await;
    let (b, s) = (bot.clone(), rt.sessions.clone());
    tokio::spawn(async move { reply::send_startup_notification(b, s).await; });

    let media_group_manager = Arc::new(super::media_group::MediaGroupManager::new());
    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(callbacks::handle_callback_query));

    let aux_deps = (rt.cron.clone(), rt.topic_cache.clone());
    let meta_deps = (bot_info.clone(), media_group_manager.clone());
    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![
            rt.cfg,
            rt.sessions,
            rt.cli,
            rt.cron,
            rt.topic_cache,
            bot_info,
            media_group_manager,
            aux_deps,
            meta_deps
        ])
        .build()
        .dispatch()
        .await;
}

/// Executes `run_bot` (starting Web/control-plane services and optional Telegram dispatcher).
pub async fn run_bot(mut config: CliConfig, paths: TunerPaths) -> Result<(), String> {
    let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or_else(|_| config.telegram_token.clone());
    if tok.trim().is_empty() || tok == "YOUR_BOT_TOKEN_HERE" || tok.starts_with("YOUR_") {
        config.transport = "web".to_string();
    }
    let rt = init_core_runtime(config, &paths).await;
    start_background_schedulers(&rt, &paths);
    if let Some((bot, bot_info)) = try_init_telegram_bot(tok, paths.profile.clone()).await {
        run_telegram_dispatcher(rt, bot, bot_info).await;
    } else {
        println!("🖥️ [tuner] Web-Standalone Dashboard active on http://127.0.0.1:8743/dashboard (run 'tuner --dashboard' for Magic Link).");
        std::future::pending::<()>().await;
    }
    Ok(())
}
