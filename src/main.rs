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
pub mod setup;


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
        setup::install_systemd_service(&config)?;
    }
    
    println!("🎉 Setup complete! Please start the tuner bot and send a message (e.g. /start) to register as owner.");
    Ok(())
}

// load_env_file moved to config module

async fn spawn_worker(current_exe: &std::path::Path, name: &str) -> Result<tokio::process::Child, String> {
    tokio::process::Command::new(current_exe)
        .arg("--worker")
        .arg(name)
        .env_remove("TELEGRAM_TOKEN")
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to spawn worker '{}': {}", name, e))
}

async fn check_restart_marker(marker: &std::path::Path, workers: &mut [(String, tokio::process::Child)]) {
    if marker.exists() {
        let _ = std::fs::remove_file(marker);
        println!("🤖 [tuner] Master detected restart request. Terminating all workers...");
        for (name, child) in workers.iter_mut() {
            println!("🤖 [tuner] Killing worker: {}", name);
            let _ = child.kill().await;
        }
        std::process::exit(42);
    }
}

async fn monitor_workers(workers: &mut [(String, tokio::process::Child)], current_exe: &std::path::Path) -> Result<(), String> {
    let mut exit_requested = None;
    for (name, child) in workers.iter_mut() {
        match child.try_wait() {
            Ok(Some(status)) => {
                println!("🤖 [tuner] Worker '{}' exited with status: {}", name, status);
                if let Some(code) = status.code() {
                    if code == 42 {
                        exit_requested = Some(name.clone());
                        break;
                    }
                }
                println!("🤖 [tuner] Restarting worker: {}", name);
                *child = spawn_worker(current_exe, name).await?;
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("🤖 [tuner] Error checking status of worker '{}': {}", name, e);
            }
        }
    }

    if let Some(ref requester) = exit_requested {
        println!("🤖 [tuner] Worker '{}' requested restart. Exiting master...", requester);
        for (name, child) in workers.iter_mut() {
            if name != requester {
                let _ = child.kill().await;
            }
        }
        std::process::exit(42);
    }
    Ok(())
}

async fn run_master_mode(config: config::CliConfig) -> Result<(), String> {
    println!("🤖 [tuner] Starting in Master Mode (Supervisor)...");
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Failed to resolve current binary path: {}", e))?;

    let mut workers = Vec::new();
    for profile in &config.profiles {
        println!("🤖 [tuner] Spawning worker for profile: {}", profile.name);
        let child = spawn_worker(&current_exe, &profile.name).await?;
        workers.push((profile.name.clone(), child));
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let marker = std::path::PathBuf::from(home).join(".tuner/restart-requested");
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));

    loop {
        interval.tick().await;
        check_restart_marker(&marker, &mut workers).await;
        monitor_workers(&mut workers, &current_exe).await?;
    }
}

fn override_profile_config(config: &mut config::CliConfig, profile_name: &str, paths: &workspace::paths::DuctorPaths) -> Result<(), String> {
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

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let worker_profile = args.iter().position(|arg| arg == "--worker")
        .and_then(|pos| args.get(pos + 1).cloned());

    let tuner_home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string())).join(".tuner");
    let paths = workspace::paths::resolve_paths(Some(tuner_home), None, None, worker_profile.clone());

    if args.contains(&"--setup".to_string()) {
        return run_setup_wizard(&paths);
    }

    let _ = setup::load_env_file(&paths.env_file());
    workspace::init::init_workspace(&paths)?;

    let mut config = config::CliConfig::load_from_file(&paths.config_path())?;
    config.working_dir = paths.workspace().clone();

    if let Some(ref profile_name) = worker_profile {
        override_profile_config(&mut config, profile_name, &paths)?;
    }

    let startup_lang = config.language.as_deref().unwrap_or("en");
    i18n::init(startup_lang);

    if args.contains(&"--install-systemd".to_string()) {
        return setup::install_systemd_service(&config);
    }
    if args.contains(&"--supervisor".to_string()) {
        let current_exe = std::env::current_exe()
            .map_err(|e| format!("Failed to resolve current binary path: {}", e))?;
        let filtered_args: Vec<String> = args.into_iter().skip(1).filter(|arg| arg != "--supervisor").collect();
        let supervisor = supervisor::Supervisor::with_args(current_exe, filtered_args);
        return supervisor.run().await;
    }

    if let Some(ref p) = worker_profile {
        println!("🤖 [tuner] Starting Telegram bot worker for '{}'...", p);
        telegram::run_bot(config, paths).await
    } else {
        run_master_mode(config).await
    }
}

// install_systemd_service moved to config module
