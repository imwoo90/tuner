//! # Telegram Bot Commands Registry and Metadata
//!
//! ## Overview
//! Registers available slash commands with Telegram Bot API across private, group, and per-chat scopes,
//! dynamically adapting workflow slash commands to the active CLI provider (`antigravity`, `claude`, `codex`).
//!
//! ## Search Tags
//! #commands-registry, #slash-commands, #command-scope, #provider-commands

use teloxide::prelude::*;

const COMMON_COMMANDS: &[(&str, &str)] = &[
    ("new", "Start a fresh conversation session"),
    ("reset", "Alias for /new"),
    ("stop", "Cancel active CLI processes in chat"),
    ("abort", "Forcefully stop all running workers"),
    ("provider", "Select or change active CLI provider (agy|claude|codex)"),
    ("model", "Select or change active AI model"),
    ("effort", "Select or change reasoning effort (minimal|low|medium|high|xhigh|max)"),
    ("compact", "Condense conversation history to free context"),
    ("usage", "Show model quota and remaining limits"),
    ("lang", "Change active language for this session"),
    ("status", "Show bot status and diagnostics report"),
    ("memory", "Print workspace MAINMEMORY.md contents"),
    ("cron", "Manage scheduled cron tasks"),
    ("remote", "Manage remote dev services"),
    ("vault", "Manage per-profile site login credentials"),
    ("upgrade", "Check for updates and perform self-upgrade"),
    ("restart", "Trigger clean restart of tuner service"),
];

const ANTIGRAVITY_WORKFLOW_COMMANDS: &[(&str, &str)] = &[
    ("plan", "Request step-by-step plan before execution"),
    ("grill_me", "Start interactive interview alignment"),
    ("goal", "Launch long-running thorough task"),
    ("teamwork", "Launch collaborative multi-agent teamwork"),
    ("boost", "Launch deep reasoning & multi-perspective pipeline"),
    ("learn", "Record learning or behavior correction"),
    ("browser", "Launch browser automation workflow"),
    ("schedule", "Schedule recurring or one-shot task"),
];

const CLAUDE_WORKFLOW_COMMANDS: &[(&str, &str)] = &[
    ("review", "Review a pull request or code changes"),
    ("security_review", "Audit pending changes for security vulnerabilities"),
    ("verify", "Verify changes against tests and requirements"),
    ("debug", "Investigate and fix errors or failing tests"),
    ("simplify", "Simplify and refactor complex code"),
    ("deep_research", "Conduct deep multi-source codebase research"),
    ("code_review", "Run automated multi-pass code review"),
    ("goal", "Launch long-running thorough task"),
    ("init", "Initialize CLAUDE.md project instructions"),
];

const CODEX_WORKFLOW_COMMANDS: &[(&str, &str)] = &[
    ("review", "Run code review over workspace changes"),
    ("init", "Initialize AGENTS.md project instructions"),
];

pub(crate) fn split_slash_command(text: &str) -> (&str, &str) {
    let trimmed = text.trim();
    if !trimmed.starts_with('/') {
        return ("", trimmed);
    }
    let first_token = trimmed.split_whitespace().next().unwrap_or("");
    let cmd = first_token.split('@').next().unwrap_or(first_token);
    let args = trimmed[first_token.len()..].trim();
    (cmd, args)
}

pub(crate) fn is_lock_free_command(text: &str) -> bool {
    let (cmd, args) = split_slash_command(text);
    args.is_empty()
        && matches!(
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
                | "/remote"
                | "/vault"
        )
}

pub(crate) fn workflow_commands_for_provider(provider: &str) -> &'static [(&'static str, &'static str)] {
    match provider {
        "claude" => CLAUDE_WORKFLOW_COMMANDS,
        "codex" => CODEX_WORKFLOW_COMMANDS,
        _ => ANTIGRAVITY_WORKFLOW_COMMANDS,
    }
}

pub(crate) fn get_bot_commands_for_provider(provider: &str) -> Vec<teloxide::types::BotCommand> {
    COMMON_COMMANDS
        .iter()
        .chain(workflow_commands_for_provider(provider).iter())
        .map(|(c, d)| teloxide::types::BotCommand {
            command: (*c).to_string(),
            description: (*d).to_string(),
        })
        .collect()
}

pub(crate) fn merge_claude_bot_commands(dynamic: Vec<(String, String)>) -> Vec<teloxide::types::BotCommand> {
    if dynamic.is_empty() {
        return get_bot_commands_for_provider("claude");
    }
    let mut cmds: Vec<teloxide::types::BotCommand> = COMMON_COMMANDS
        .iter()
        .map(|(c, d)| teloxide::types::BotCommand { command: (*c).to_string(), description: (*d).to_string() })
        .collect();
    for (c, d) in dynamic.into_iter().chain(CLAUDE_WORKFLOW_COMMANDS.iter().map(|(c, d)| ((*c).to_string(), (*d).to_string()))) {
        if cmds.len() >= 100 { break; }
        if !cmds.iter().any(|b| b.command == c) {
            cmds.push(teloxide::types::BotCommand { command: c, description: d });
        }
    }
    cmds
}

static LAST_SYNCED_CHAT_PROVIDER: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<i64, String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

pub(crate) fn get_bot_commands() -> Vec<teloxide::types::BotCommand> {
    get_bot_commands_for_provider("antigravity")
}

pub(crate) async fn update_chat_commands_for_provider(bot: &Bot, chat_id: ChatId, provider: &str) {
    if let Ok(mut map) = LAST_SYNCED_CHAT_PROVIDER.lock() {
        map.insert(chat_id.0, provider.to_string());
    }
    let cmds = if provider == "claude" && !cfg!(test) {
        let env: std::collections::HashMap<_, _> = std::env::vars().collect();
        merge_claude_bot_commands(crate::cli::claude::discover_commands_live(&env).await)
    } else {
        get_bot_commands_for_provider(provider)
    };
    let _ = bot.set_my_commands(cmds).scope(teloxide::types::BotCommandScope::Chat { chat_id: chat_id.into() }).await;
}

pub(crate) async fn sync_chat_commands_for_topic(
    bot: &Bot,
    msg: &Message,
    sessions: &crate::session::manager::SessionManager,
    config: &crate::config::CliConfig,
) {
    if cfg!(test) { return; }
    let key = crate::session::key::SessionKey::telegram(msg.chat.id.0, crate::telegram::get_topic_id(msg));
    let prov = sessions.get_active(&key).await.ok().flatten().map(|s| s.provider).filter(|p| !p.is_empty()).unwrap_or_else(|| config.provider.clone());
    if prov.is_empty() { return; }
    if LAST_SYNCED_CHAT_PROVIDER.lock().ok().is_some_and(|m| m.get(&msg.chat.id.0).map(String::as_str) != Some(prov.as_str())) {
        let (b, cid) = (bot.clone(), msg.chat.id);
        tokio::spawn(async move { update_chat_commands_for_provider(&b, cid, &prov).await; });
    }
}

pub(crate) fn is_workflow_slash_command(text: &str) -> bool {
    let (cmd, _) = split_slash_command(text);
    matches!(
        cmd,
        "/goal" | "/plan" | "/learn" | "/grill_me" | "/grill-me" | "/teamwork" | "/teamwork_preview"
        | "/teamwork-preview" | "/browser" | "/boost" | "/schedule" | "/compact" | "/review"
        | "/security_review" | "/security-review" | "/verify" | "/debug" | "/simplify"
        | "/deep_research" | "/deep-research" | "/code_review" | "/code-review" | "/init" | "/clear"
    )
}

pub(crate) fn escape_non_workflow_slash(text: &str) -> String {
    let trimmed = text.trim_start();
    if trimmed.starts_with('/') && !is_workflow_slash_command(trimmed) { format!(" {}", text) } else { text.to_string() }
}

pub(crate) async fn register_commands(bot: &Bot) -> Result<(), teloxide::RequestError> {
    let cmds = get_bot_commands();
    for s in [None, Some(teloxide::types::BotCommandScope::AllPrivateChats), Some(teloxide::types::BotCommandScope::AllGroupChats)] {
        if let Some(sc) = s.clone() { let _ = bot.delete_my_commands().scope(sc).await; } else { let _ = bot.delete_my_commands().await; }
        if let Some(sc) = s { let _ = bot.set_my_commands(cmds.clone()).scope(sc).await; } else { let _ = bot.set_my_commands(cmds.clone()).await; }
    }
    Ok(())
}

pub(crate) fn format_help_text_for_provider(provider: &str) -> String {
    let mut text = String::from("🤖 <b>[tuner] Command Reference:</b>\n\n");
    for (cmd, desc) in COMMON_COMMANDS {
        text.push_str(&format!("• /{} - {}\n", cmd, desc));
    }
    text.push_str(&format!("\n⚡ <b>Workflow Commands ({}):</b>\n", provider));
    for (cmd, desc) in workflow_commands_for_provider(provider) {
        text.push_str(&format!("• /{} - {}\n", cmd, desc));
    }
    text
}

pub(crate) async fn handle_help_command_for_provider(bot: &Bot, msg: &Message, provider: &str) -> Result<(), teloxide::RequestError> {
    let mut req = bot.send_message(msg.chat.id, format_help_text_for_provider(provider)).parse_mode(teloxide::types::ParseMode::Html);
    if let Some(tid) = msg.thread_id { req = req.message_thread_id(tid); }
    let _ = req.await;
    Ok(())
}

#[allow(dead_code)]
pub(crate) async fn handle_help_command(bot: &Bot, msg: &Message) -> Result<(), teloxide::RequestError> {
    handle_help_command_for_provider(bot, msg, "antigravity").await
}
