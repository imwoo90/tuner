//! # Filesystem Synchronization
//!
//! Handles creating directory layouts, deploying framework-managed files (Zone 2),
//! seeding defaults (Zone 3), merging user config, rule sync, and watcher loops.

use crate::workspace::paths::DuctorPaths;
use crate::workspace::sync_helpers::{smart_merge_config, sync_group, sync_rule_files_recursive, walk_and_copy};
use std::path::Path;

static DOCKER_NOTICE: &str = "\n\n---\n\n## Runtime Environment\n\n**IMPORTANT: YOU ARE RUNNING INSIDE A DOCKER CONTAINER (`{container}`).**\n\n- Your filesystem is isolated. `/ductor` is the mounted host directory `~/.tuner`.\n- You cannot see or access the host system outside this mount.\n- Feel free to experiment -- the host is protected.\n";

static HOST_NOTICE: &str = "\n\n---\n\n## Runtime Environment\n\n**WARNING: YOU ARE RUNNING DIRECTLY ON THE HOST SYSTEM. THERE IS NO SANDBOX.**\n\n- Every file operation, command, and script runs on the user's real machine.\n- Be careful with destructive commands (`rm -rf`, `chmod`, etc.).\n- Ask before touching anything outside `workspace/`.\n";

static TRANSPORT_TELEGRAM: &str = "\n\n---\n\n## Messenger Rules\n\n- Replies are Telegram messages (4096-char limit; auto-split is handled).\n- Keep responses mobile-friendly and structured.\n- To send files, use `<file:/absolute/path>`.\n- Save generated deliverables in `output_to_user/`.\n- Do not suggest GUI-only actions like `xdg-open`.\n\n### Quick Reply Buttons\n\nUse button syntax at the end of messages:\n\n- `[button:Label]` markers\n- same line = one row\n- new line = new row\n\nKeep labels short. Callback data is truncated to 64 bytes by the framework.\nDo not place button markers inside code blocks.\n";

static IDENTITY_MAIN: &str = "\n\n---\n\n## Multi-Agent Identity\n\n**You are the MAIN agent (`{name}`).**\n\n- You are the coordinator in a multi-agent system.\n- Each sub-agent has its own bot/chat.\n\n### How the user interacts with sub-agents\n\n1. **Direct chat**: The user opens the sub-agent's bot and chats directly.\n2. **Delegation via you**: The user asks YOU to delegate a task using the agent tools below.\n\nAfter creating a sub-agent, tell the user they can open its chat directly. Do not suggest internal tools to the user.\n\n### Agent tools (for YOUR internal use)\n\n- `python3 tools/agent_tools/ask_agent.py TARGET \"message\"`\n- `python3 tools/agent_tools/ask_agent_async.py TARGET \"message\"`\n- `python3 tools/agent_tools/list_agents.py`\n- `python3 tools/agent_tools/edit_shared_knowledge.py`\n\nResponses come back to YOU, never to the sub-agent. Use async for tasks taking more than a few seconds.\n\nAsynchronous sub-agent tasks run in a session called `ia-{name}`. The user can follow up directly via `@ia-{name} <message>`. Mention this session name when reporting results.\n";

fn migrate_legacy_data(paths: &DuctorPaths) {
    if let Some(ref p) = paths.profile {
        if p == "default" {
            let legacy_sessions = paths.tuner_home.join("sessions.json");
            let legacy_workspace = paths.tuner_home.join("workspace");
            let target_sessions = paths.sessions_path();
            let target_workspace = paths.workspace();

            if legacy_sessions.is_file() && !target_sessions.exists() {
                if let Some(parent) = target_sessions.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::rename(&legacy_sessions, &target_sessions);
            }
            if legacy_workspace.is_dir() && !target_workspace.exists() {
                if let Some(parent) = target_workspace.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::rename(&legacy_workspace, &target_workspace);
            }
        }
    }
}

fn create_workspace_directories(paths: &DuctorPaths) {
    let required_workspace_dirs = [
        "",
        "memory_system",
        "cron_tasks",
        "tools",
        "tools/user_tools",
        "tools/cron_tools",
        "tools/media_tools",
        "tools/webhook_tools",
        "output_to_user",
        "tasks",
        "skills",
    ];
    for rel in &required_workspace_dirs {
        let d = if rel.is_empty() {
            paths.workspace()
        } else {
            paths.workspace().join(rel)
        };
        if !d.is_dir() {
            let _ = std::fs::create_dir_all(&d);
        }
    }
    let _ = std::fs::create_dir_all(paths.config_dir());
    let _ = std::fs::create_dir_all(paths.logs_dir());
}

/// Initializes the workspace directory structure and configurations.
pub fn init_workspace(paths: &DuctorPaths) -> Result<(), String> {
    migrate_legacy_data(paths);

    let old_tasks = paths.workspace().join("tasks");
    if old_tasks.is_dir() && !paths.cron_tasks_dir().exists() {
        let _ = std::fs::rename(&old_tasks, paths.cron_tasks_dir());
    }

    let _ = crate::workspace::skills::sync_bundled_skills(paths, false);

    if paths.home_defaults.is_dir() {
        walk_and_copy(&paths.home_defaults, &paths.tuner_home, &paths.home_defaults)?;
    }

    create_workspace_directories(paths);

    let selector = crate::workspace::rules::RulesSelector::new(paths.clone());
    let _ = selector.deploy_rules();

    let _ = ensure_task_rule_files(&paths.cron_tasks_dir());
    let _ = sync_rule_files(&paths.workspace());
    if paths.profile.is_none() {
        let _ = smart_merge_config(paths);
    }

    if paths.workspace().is_dir() {
        if let Ok(entries) = std::fs::read_dir(paths.workspace()) {
            for entry in entries {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.is_symlink() && !path.exists() {
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }
    }

    let _ = crate::workspace::skills::sync_skills(paths, false);
    Ok(())
}

/// Injects runtime environment notices into CLAUDE.md.
pub fn inject_runtime_environment(
    paths: &DuctorPaths,
    docker_container: Option<&str>,
) -> Result<(), String> {
    let env_notice = if let Some(container) = docker_container {
        DOCKER_NOTICE.replace("{container}", container)
    } else {
        HOST_NOTICE.to_string()
    };

    let identity_notice = IDENTITY_MAIN.replace("{name}", "main");
    let transport_notice = TRANSPORT_TELEGRAM.to_string();

    let rule_filenames = ["CLAUDE.md", "AGENTS.md", "GEMINI.md"];
    for name in &rule_filenames {
        let target = paths.workspace().join(name);
        if target.is_file() {
            if let Ok(content) = std::fs::read_to_string(&target) {
                if content.contains("## Multi-Agent Identity") || content.contains("## Runtime Environment") {
                    continue;
                }
                let new_content = format!("{}{}{}{}", content, transport_notice, identity_notice, env_notice);
                let _ = std::fs::write(&target, new_content);
            }
        }
    }
    Ok(())
}

/// Recursively synchronizes existing rule files across the workspace.
pub fn sync_rule_files(root: &Path) -> Result<(), String> {
    if !root.is_dir() {
        return Ok(());
    }
    sync_group(root)?;
    sync_rule_files_recursive(root)?;
    Ok(())
}

/// Asynchronously monitors changes to rule files and syncs them periodically.
pub async fn watch_rule_files(root: &Path, interval_ms: u64) -> Result<(), String> {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(interval_ms));
    let cron_tasks_dir = root.join("cron_tasks");
    loop {
        interval.tick().await;
        let root_clone = root.to_path_buf();
        let cron_tasks_clone = cron_tasks_dir.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let _ = ensure_task_rule_files(&cron_tasks_clone);
            let _ = sync_rule_files(&root_clone);
        }).await;
    }
}

pub fn ensure_task_rule_files(cron_tasks_dir: &Path) -> Result<usize, String> {
    if !cron_tasks_dir.is_dir() {
        return Ok(0);
    }
    let expected = detect_rule_filenames(cron_tasks_dir);
    let mut created = 0;
    let rule_filenames = ["CLAUDE.md", "AGENTS.md", "GEMINI.md"];

    let entries = match std::fs::read_dir(cron_tasks_dir) {
        Ok(e) => e,
        Err(e) => return Err(format!("Failed to read dir: {}", e)),
    };

    for entry in entries {
        if let Ok(entry) = entry {
            let task_dir = entry.path();
            if task_dir.is_dir() {
                let mut existing = Vec::new();
                for name in &rule_filenames {
                    if task_dir.join(name).is_file() {
                        existing.push(*name);
                    }
                }
                if existing.is_empty() {
                    continue;
                }
                let mut missing = Vec::new();
                for name in &expected {
                    if !task_dir.join(name).is_file() {
                        missing.push(name.clone());
                    }
                }
                if missing.is_empty() {
                    continue;
                }
                if let Ok(source_content) = std::fs::read_to_string(task_dir.join(existing[0])) {
                    for name in missing {
                        let _ = std::fs::write(task_dir.join(name), &source_content);
                        created += 1;
                    }
                }
            }
        }
    }
    Ok(created)
}

fn detect_rule_filenames(cron_tasks_dir: &Path) -> Vec<String> {
    let rule_filenames = ["CLAUDE.md", "AGENTS.md", "GEMINI.md"];
    let mut found = Vec::new();
    for name in &rule_filenames {
        if cron_tasks_dir.join(name).is_file() {
            found.push(name.to_string());
        }
    }
    if found.is_empty() {
        vec!["CLAUDE.md".to_string()]
    } else {
        found
    }
}
