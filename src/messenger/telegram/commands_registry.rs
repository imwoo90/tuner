//! # Telegram Bot Commands Registry and Metadata
//!
//! Registers available slash commands with Telegram Bot API across private and group chat scopes,
//! and provides classification for lock-free command dispatch.

use teloxide::prelude::*;

pub(crate) fn is_lock_free_command(text: &str) -> bool {
    let cmd = text.split_whitespace().next().unwrap_or("");
    matches!(
        cmd,
        "/status"
            | "/help"
            | "/start"
            | "/diagnose"
            | "/memory"
            | "/cron"
            | "/upgrade"
            | "/restart"
            | "/stop"
            | "/stop_all"
            | "/abort"
    )
}

pub(crate) fn get_bot_commands() -> Vec<teloxide::types::BotCommand> {
    let list = [
        ("new", "Start a fresh conversation session"),
        ("reset", "Alias for /new"),
        ("stop", "Cancel active CLI processes in chat"),
        ("abort", "Forcefully stop all running workers"),
        ("model", "Select or change active AI model"),
        ("effort", "Select or change reasoning effort (low|medium|high)"),
        ("lang", "Change active language for this session"),
        ("status", "Show bot status and diagnostics report"),
        ("memory", "Print workspace MAINMEMORY.md contents"),
        ("restart", "Trigger clean restart of tuner service"),
        ("plan", "Request step-by-step plan before execution"),
        ("grill_me", "Start interactive interview alignment"),
        ("goal", "Launch long-running thorough task"),
        ("learn", "Record learning or behavior correction"),
        ("teamwork_preview", "Launch collaborative multi-agent simulation"),
        ("upgrade", "Check for updates and perform self-upgrade"),
    ];
    list.into_iter().map(|(c, d)| teloxide::types::BotCommand {
        command: c.to_string(),
        description: d.to_string(),
    }).collect()
}

pub(crate) async fn register_commands(bot: &Bot) -> Result<(), teloxide::RequestError> {
    let cmds = get_bot_commands();
    let _ = bot.delete_my_commands().await;
    let _ = bot.delete_my_commands().scope(teloxide::types::BotCommandScope::AllPrivateChats).await;
    let _ = bot.delete_my_commands().scope(teloxide::types::BotCommandScope::AllGroupChats).await;

    let _ = bot.set_my_commands(cmds.clone()).await;
    let _ = bot.set_my_commands(cmds.clone()).scope(teloxide::types::BotCommandScope::AllPrivateChats).await;
    let _ = bot.set_my_commands(cmds).scope(teloxide::types::BotCommandScope::AllGroupChats).await;
    Ok(())
}
