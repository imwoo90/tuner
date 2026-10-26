//! # Wizard and Service Installer Module
//!
//! ## Overview
//! Operates the wizard setup CLI prompt. Installs systemd user services.
//!
//! ## Search Tags
//! #wizard-installer, #systemd-setup, #env-loader

use crate::config::CliConfig;
use std::path::Path;

/// Executes `load_env_file` (load env file).
pub fn load_env_file(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Ok(());
    }
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let line_to_parse = if let Some(rest) = trimmed.strip_prefix("export ") {
            rest.trim()
        } else {
            trimmed
        };
        if let Some(pos) = line_to_parse.find('=') {
            let key = line_to_parse[..pos].trim();
            let mut val = line_to_parse[pos + 1..].trim().to_string();
            if ((val.starts_with('"') && val.ends_with('"')) || (val.starts_with('\'') && val.ends_with('\'')))
                && val.len() >= 2 {
                    val = val[1..val.len() - 1].to_string();
                }
            if !key.is_empty() && std::env::var(key).is_err() {
                unsafe { std::env::set_var(key, val); }
            }
        }
    }
    Ok(())
}

/// Executes `install_systemd_service` (install systemd service).
pub fn install_systemd_service(config: &CliConfig) -> Result<(), String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let project_root = current_exe.parent().and_then(|p| p.parent()).and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));
    let path_env = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".to_string());

    let token_line = if let Ok(tok) = std::env::var("TELEGRAM_TOKEN") {
        format!("Environment=\"TELEGRAM_TOKEN={}\"\n", tok)
    } else if !config.telegram_token.is_empty() && config.telegram_token != "YOUR_BOT_TOKEN_HERE" {
        format!("Environment=\"TELEGRAM_TOKEN={}\"\n", config.telegram_token)
    } else {
        String::new()
    };

    let unit_content = format!(
        "[Unit]\nDescription=Tuner Bot\nAfter=network.target\n\n[Service]\nType=simple\n\
         WorkingDirectory={}\nExecStart={}\n{}Environment=\"HOME={}\"\nEnvironment=\"PATH={}\"\n\
         Restart=always\nRestartSec=10\nOOMPolicy=continue\n\n\
         [Install]\nWantedBy=default.target\n",
        project_root.to_string_lossy(), current_exe.to_string_lossy(), token_line, home, path_env
    );
    let systemd_dir = std::path::PathBuf::from(&home).join(".config/systemd/user");
    std::fs::create_dir_all(&systemd_dir).map_err(|e| e.to_string())?;
    let service_file = systemd_dir.join("tuner.service");
    std::fs::write(&service_file, unit_content).map_err(|e| e.to_string())?;
    println!("🤖 [tuner] Installed successfully to {:?}", service_file);

    install_agy_vault_systemd_if_present(&home, &systemd_dir);

    println!("💡 Run: systemctl --user daemon-reload && systemctl --user restart tuner");
    Ok(())
}

/// Locates the `agy-vault` companion binary in standard user/system directories.
pub fn find_agy_vault_bin(home: &str) -> Option<std::path::PathBuf> {
    let candidates = [
        std::path::PathBuf::from(home).join(".local/bin/agy-vault"),
        std::path::PathBuf::from(home).join(".tuner/bin/agy-vault"),
        std::path::PathBuf::from("/usr/local/bin/agy-vault"),
        std::path::PathBuf::from("/usr/bin/agy-vault"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

fn install_agy_vault_systemd_if_present(home: &str, systemd_dir: &Path) {
    let agy_dir = std::path::PathBuf::from(home).join(".gemini/antigravity-cli");
    let Some(vault_bin) = find_agy_vault_bin(home) else {
        return;
    };
    if !agy_dir.is_dir() {
        return;
    }
    let vault_service = systemd_dir.join("agy-vault.service");
    if vault_service.exists() {
        println!("🗄️ [tuner] Existing companion service preserved at {:?}", vault_service);
        return;
    }
    let unit = format!(
        "[Unit]\nDescription=Zero-Knowledge Antigravity Session Preservation & Auto-Restore Daemon (agy-vault)\n\
         After=default.target\nStartLimitIntervalSec=0\n\n[Service]\nType=simple\n\
         ExecStart={} daemon --poll-ms 1000\nRestart=always\nRestartSec=3\n\
         Nice=5\nOOMScoreAdjust=-500\n\n[Install]\nWantedBy=default.target\n",
        vault_bin.to_string_lossy()
    );
    if std::fs::write(&vault_service, unit).is_ok() {
        println!("🗄️ [tuner] Companion service installed to {:?}", vault_service);
    }
}

/// Returns `true` if an `agy-vault daemon` process is already running in `/proc`.
pub fn is_agy_vault_daemon_running() -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) {
            let text = String::from_utf8_lossy(&cmdline);
            if text.contains("agy-vault") && text.contains("daemon") {
                return true;
            }
        }
    }
    false
}

/// Ensures `agy-vault` binary patch and companion daemon are active when using Antigravity.
pub fn ensure_agy_vault_companion(config: &CliConfig) {
    if config.provider != "antigravity" {
        return;
    }
    if std::env::var("TUNER_DISABLE_AGY_VAULT").ok().as_deref() == Some("1") {
        return;
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let agy_dir = std::path::PathBuf::from(&home).join(".gemini/antigravity-cli");
    if !agy_dir.is_dir() {
        return;
    }
    let Some(vault_bin) = find_agy_vault_bin(&home) else {
        return;
    };
    if is_agy_vault_daemon_running() {
        println!("🗄️ [tuner] agy-vault companion daemon is already active.");
        return;
    }
    let _ = std::process::Command::new(&vault_bin)
        .arg("patch-binary")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match std::process::Command::new(&vault_bin)
        .args(["daemon", "--poll-ms", "1000"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => println!(
            "🗄️ [tuner] Spawned agy-vault companion daemon (PID: {}, bin: {:?})",
            child.id(),
            vault_bin
        ),
        Err(e) => eprintln!("⚠️ [tuner] Failed to spawn agy-vault companion daemon: {}", e),
    }
}

/// Executes `override_profile_config` (override profile config).
pub fn override_profile_config(
    config: &mut CliConfig,
    profile_name: &str,
    paths: &crate::workspace::paths::TunerPaths,
) -> Result<(), String> {
    let profile_cfg = config.profiles.iter().find(|p| p.name == profile_name)
        .ok_or_else(|| format!("Profile '{}' not found in config.json", profile_name))?;
    if !profile_cfg.telegram_token.is_empty() && profile_cfg.telegram_token != "YOUR_BOT_TOKEN_HERE" && !profile_cfg.telegram_token.starts_with("YOUR_") {
        config.telegram_token = profile_cfg.telegram_token.clone();
    } else if profile_name != "default" {
        config.telegram_token = String::new();
    }
    if !profile_cfg.allowed_user_ids.is_empty() && profile_cfg.allowed_user_ids != vec![123456789] {
        config.allowed_user_ids = profile_cfg.allowed_user_ids.clone();
    }
    if !profile_cfg.allowed_group_ids.is_empty() && profile_cfg.allowed_group_ids != vec![-1001234567890] {
        config.allowed_group_ids = profile_cfg.allowed_group_ids.clone();
    }
    config.working_dir = profile_cfg.working_dir.clone().unwrap_or_else(|| paths.workspace());
    if let Some(ref m) = profile_cfg.model {
        config.model = Some(m.clone());
    }
    if let Some(ref e) = profile_cfg.effort {
        config.effort = Some(e.clone());
    }
    if let Some(ref p) = profile_cfg.system_prompt {
        config.system_prompt = Some(p.clone());
    }
    if let Some(ref p) = profile_cfg.append_system_prompt {
        config.append_system_prompt = Some(p.clone());
    }
    if let Some(ref l) = profile_cfg.language {
        config.language = Some(l.clone());
    }
    Ok(())
}
