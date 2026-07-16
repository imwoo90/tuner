
use teloxide::prelude::*;
use crate::config::CliConfig;
use crate::cli::{antigravity::AntigravityCli, AgentProvider};
use crate::session::{manager::SessionManager, data::SessionData};
use std::sync::Arc;

pub mod formatting;
#[cfg(test)]
pub mod formatting_tests;
pub mod reply;
#[cfg(test)]
pub mod reply_tests;
#[cfg(test)]
pub mod handler_tests;
#[cfg(test)]
pub mod ask_abort_tests;
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


pub use reply::get_topic_id;

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
    let _g = typing::TelegramTypingGuard::new(bot.clone(), tok, msg).await;
    let mut cc = cli.clone();
    if !sess.model.is_empty() { cc.config.model = Some(sess.model.clone()); }
    match cc.send_streaming(prompt, (!sid.is_empty()).then_some(sid), false, config.working_dir.clone()).await {
        Ok(s) => stream::consume_stream(bot, msg.chat.id, msg.thread_id, s, sessions, sess, config, cli).await?,
        Err(e) => {
            eprintln!("CLI ERROR: {:?}", e);
            let mut r = bot.send_message(msg.chat.id, format!("❌ Error: {}", e));
            if let Some(t) = msg.thread_id { r = r.message_thread_id(t); }
            let _ = r.await;
        }
    }
    Ok(())
}

async fn handle_model_override(
    bot: &Bot,
    msg: &Message,
    mo: &str,
    sess: &mut crate::session::data::SessionData,
    sessions: &SessionManager,
    empty: bool,
) -> Result<bool, teloxide::RequestError> {
    sess.model = mo.to_string();
    let _ = sessions.update_session(sess, 0.0, 0).await;
    if empty {
        let mut r = bot.send_message(msg.chat.id, format!("Next message will use {}", mo));
        if let Some(t) = msg.thread_id { r = r.message_thread_id(t); }
        let _ = r.await;
        return Ok(true);
    }
    Ok(false)
}

async fn feed_active_session_if_running(
    bot: &Bot,
    chat_id: teloxide::types::ChatId,
    session_id: &str,
    current_text: &str,
    cli: &AntigravityCli,
) -> Result<bool, teloxide::RequestError> {
    if cli.sessions.is_active(session_id).await && cli.sessions.is_running(session_id).await {
        let mut input_prompt = format!("{}\r", current_text);
        if cli.sessions.is_ask_active(session_id).await {
            let msg_id = cli.sessions.get_ask_msg_id(session_id).await;
            if let Some(opts) = cli.sessions.get_ask_options(session_id).await {
                if let Some(idx) = formatting::find_best_option(current_text, &opts) {
                    input_prompt = if idx == 0 { "\r".to_string() } else { format!("{}\r", "j".repeat(idx)) };
                    println!("matched option {}: {:?}", idx, opts[idx]);
                }
            }
            cli.sessions.set_ask_active(session_id, false).await;
            if let Some(mid) = msg_id {
                let _ = bot.edit_message_reply_markup(chat_id, teloxide::types::MessageId(mid)).await;
            }
        }
        println!("feed: {} {:?}", session_id, input_prompt);
        let _ = cli.sessions.write_to_session(session_id, &input_prompt).await;
        return Ok(true);
    }
    Ok(false)
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

    let (m_over, current_text) = parse_model_directive(text);
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, get_topic_id(msg));
    let mut m = config.model.clone().unwrap_or_else(|| "antigravity-default".to_string());
    if let Some(ref mo) = m_over { m = mo.clone(); }

    let (mut sess, _) = sessions.resolve_session(&key, &config.provider, &m).await.unwrap();
    if let Some(ref mo) = m_over {
        if handle_model_override(bot, msg, mo, &mut sess, sessions, current_text.is_empty()).await? {
            return Ok(());
        }
    }

    let mut prompt = build_reply_prompt(msg, current_text);
    let _ = reply::download_and_inject_media_hint(bot, msg, &config.working_dir, &mut prompt).await;

    let session_id = sess.get_session_id(&config.provider);
    if feed_active_session_if_running(bot, msg.chat.id, &session_id, current_text, cli).await? {
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
    let chat_id = msg.chat.id.0;
    if let Some(to_chat) = msg.migrate_to_chat_id() {
        let _ = sessions.migrate_chat_id(chat_id, to_chat.0).await;
        return Ok(());
    }
    if topic_cache::handle_forum_topic_events(&msg, &topic_cache, chat_id) {
        return Ok(());
    }
    let ok = config.allowed_user_ids.contains(&from_id)
        && (!msg.chat.is_group() && !msg.chat.is_supergroup() || config.allowed_group_ids.contains(&chat_id));
    if ok {
        let text = reply::strip_mention(msg.text().or(msg.caption()).unwrap_or(""), bot_info.username.as_deref())
            .replace("/teamwork_preview", "/teamwork-preview").replace("/grill_me", "/grill-me");
        if !text.is_empty() || reply::has_media(&msg) {
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
    
    let active_lang = sessions.resolve_session(&key, &config.provider, default_model).await
        .map(|(s, _)| s.language)
        .ok().flatten()
        .or_else(|| config.language.clone())
        .unwrap_or_else(|| "en".to_string());

    let fut = crate::i18n::TASK_ACTIVE_LANG.scope(active_lang, async move {
        handle_message_inner(bot, msg, config, sessions, cli, cron_manager, topic_cache, bot_info).await
    });
    if cfg!(test) { fut.await } else { tokio::spawn(fut); Ok(()) }
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
    let tok = std::env::var("TELEGRAM_TOKEN").unwrap_or(config.telegram_token.clone());
    if tok.is_empty() { return Err("No token".to_string()); }
    let bot = Bot::new(tok);
    let _ = commands::register_commands(&bot).await;
    let bot_info = Arc::new(BotInfo { username: bot.get_me().await.ok().and_then(|m| m.user.username) });
    let config_arc = Arc::new(config);
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/wimvm".to_string());
    
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
