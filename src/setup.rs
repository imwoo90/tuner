//! # Wizard and Service Installer Module
//!
//! ## Overview
//! Operates the wizard setup CLI prompt. Installs systemd user services.
//!
//! ## Collaboration Graph
//! - Invoked via `--setup` or `--install-systemd` flags.
//!
//! ## Search Tags
//! #wizard-installer, #systemd-setup, #env-loader

use crate::config::CliConfig;
use std::path::Path;

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
        let line_to_parse = if trimmed.starts_with("export ") {
            trimmed["export ".len()..].trim()
        } else {
            trimmed
        };
        if let Some(pos) = line_to_parse.find('=') {
            let key = line_to_parse[..pos].trim();
            let mut val = line_to_parse[pos + 1..].trim().to_string();
            if (val.starts_with('"') && val.ends_with('"')) || (val.starts_with('\'') && val.ends_with('\'')) {
                if val.len() >= 2 {
                    val = val[1..val.len() - 1].to_string();
                }
            }
            if !key.is_empty() && std::env::var(key).is_err() {
                unsafe { std::env::set_var(key, val); }
            }
        }
    }
    Ok(())
}

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
         Restart=always\nRestartSec=10\n\n\
         [Install]\nWantedBy=default.target\n",
        project_root.to_string_lossy(), current_exe.to_string_lossy(), token_line, home, path_env
    );
    let systemd_dir = std::path::PathBuf::from(&home).join(".config/systemd/user");
    std::fs::create_dir_all(&systemd_dir).map_err(|e| e.to_string())?;
    let service_file = systemd_dir.join("tuner.service");
    std::fs::write(&service_file, unit_content).map_err(|e| e.to_string())?;
    println!("🤖 [tuner] Installed successfully to {:?}", service_file);
    println!("💡 Run: systemctl --user daemon-reload && systemctl --user restart tuner");
    Ok(())
}

pub fn override_profile_config(
    config: &mut CliConfig,
    profile_name: &str,
    paths: &crate::workspace::paths::DuctorPaths,
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
