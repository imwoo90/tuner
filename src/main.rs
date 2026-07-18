pub mod cli;
pub mod config;
pub mod telegram;
pub mod session;
pub mod cleanup;
pub mod heartbeat;
pub mod cron;
pub mod workspace;
pub mod background;
pub mod security;
pub mod bus;
pub mod webhook;
pub mod tasks;
pub mod i18n;
pub mod messenger;
pub mod supervisor;


#[cfg(test)]
pub mod telegram_tests;

#[cfg(test)]
pub mod supervisor_tests;

#[cfg(test)]
pub mod webhook_tests;

#[cfg(test)]
pub mod tasks_tests;

#[cfg(test)]
pub mod i18n_tests;

#[cfg(test)]
pub mod i18n_tests_extra;

#[cfg(test)]
pub mod i18n_check_tests;


#[cfg(test)]
pub mod cleanup_tests;

#[cfg(test)]
pub mod heartbeat_tests;

#[cfg(test)]
pub mod cron_tests;

#[cfg(test)]
pub mod workspace_tests;

#[cfg(test)]
pub mod background_tests;

#[cfg(test)]
pub mod background_tests_extra;

#[cfg(test)]
pub mod security_tests;

#[cfg(test)]
pub mod security_tests_extra;

#[cfg(test)]
pub mod bus_tests;

#[cfg(test)]
#[path = "messenger/matrix/concurrency_tests.rs"]
pub mod matrix_concurrency_tests;




use std::io::Write;

fn prompt_input(query: &str) -> Result<String, String> {
    print!("{}", query);
    std::io::stdout().flush().map_err(|e| e.to_string())?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).map_err(|e| e.to_string())?;
    Ok(input.trim().to_string())
}

fn run_setup_wizard(paths: &workspace::paths::DuctorPaths) -> Result<(), String> {
    println!("🤖 [tuner] Setup Wizard");
    workspace::init::init_workspace(paths)?;
    
    let token = loop {
        let t = prompt_input("Enter your Telegram Bot Token: ")?;
        if !t.is_empty() && t != "YOUR_BOT_TOKEN_HERE" {
            break t;
        }
        println!("❌ Token cannot be empty. Please try again.");
    };

    let content = std::fs::read_to_string(&paths.config_path()).map_err(|e| e.to_string())?;
    let mut config_val: serde_json::Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    
    if let Some(obj) = config_val.as_object_mut() {
        obj.insert("telegram_token".to_string(), serde_json::Value::String(token));
        obj.insert("allowed_user_ids".to_string(), serde_json::Value::Array(Vec::new()));
    }

    let config_content = serde_json::to_string_pretty(&config_val).map_err(|e| e.to_string())?;
    std::fs::write(&paths.config_path(), config_content).map_err(|e| e.to_string())?;
    println!("✅ Configuration saved successfully to {:?}", paths.config_path());

    let config = config::CliConfig::load_from_file(&paths.config_path())?;

    let install_sys = prompt_input("Do you want to install tuner as a systemd user service? (y/n): ")?;
    if install_sys.to_lowercase().starts_with('y') {
        install_systemd_service(&config)?;
    }
    
    println!("🎉 Setup complete! Please start the tuner bot and send a message (e.g. /start) to register as owner.");
    Ok(())
}

fn load_env_file(path: &std::path::Path) -> Result<(), String> {
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

#[tokio::main]
async fn main() -> Result<(), String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let tuner_home = std::path::PathBuf::from(&home).join(".tuner");
    let paths = workspace::paths::resolve_paths(Some(tuner_home), None, None);

    let args: Vec<String> = std::env::args().collect();
    if args.contains(&"--setup".to_string()) {
        return run_setup_wizard(&paths);
    }

    let _ = load_env_file(&paths.env_file());
    workspace::init::init_workspace(&paths)?;

    let mut config = config::CliConfig::load_from_file(&paths.config_path())?;
    config.working_dir = paths.workspace().clone();

    let startup_lang = config.language.as_deref().unwrap_or("en");
    i18n::init(startup_lang);

    if args.contains(&"--install-systemd".to_string()) {
        return install_systemd_service(&config);
    }
    if args.contains(&"--supervisor".to_string()) {
        let current_exe = std::env::current_exe()
            .map_err(|e| format!("Failed to resolve current binary path: {}", e))?;
        let filtered_args: Vec<String> = args
            .into_iter()
            .skip(1)
            .filter(|arg| arg != "--supervisor")
            .collect();
        let supervisor = supervisor::Supervisor::with_args(current_exe, filtered_args);
        return supervisor.run().await;
    }

    println!("🤖 [tuner] Loading config from: {:?}", paths.config_path());
    println!("🤖 [tuner] Starting Telegram bot...");
    telegram::run_bot(config).await?;
    
    Ok(())
}

fn install_systemd_service(config: &config::CliConfig) -> Result<(), String> {
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
