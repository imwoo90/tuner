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
pub mod upgrade;

#[cfg(test)]
pub mod telegram_tests;

#[cfg(test)]
pub mod config_tests;

#[cfg(test)]
pub mod upgrade_tests;

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



async fn handle_early_args(args: &[String]) -> Result<Option<()>, String> {
    if args.contains(&"--version".to_string()) || args.contains(&"-V".to_string()) {
        println!("tuner {}", env!("CARGO_PKG_VERSION"));
        return Ok(Some(()));
    }
    if args.contains(&"--upgrade".to_string()) {
        upgrade::run_cli_upgrade().await?;
        return Ok(Some(()));
    }
    Ok(None)
}

async fn handle_daemon_mode(args: &[String], config: &config::CliConfig) -> Result<Option<()>, String> {
    if args.contains(&"--install-systemd".to_string()) {
        setup::install_systemd_service(config)?;
        return Ok(Some(()));
    }
    if args.contains(&"--supervisor".to_string()) {
        let current_exe = std::env::current_exe()
            .map_err(|e| format!("Failed to resolve current binary path: {}", e))?;
        let filtered_args: Vec<String> = args.iter().cloned().skip(1).filter(|arg| arg != "--supervisor").collect();
        let supervisor = supervisor::Supervisor::with_args(current_exe, filtered_args);
        supervisor.run().await?;
        return Ok(Some(()));
    }
    Ok(None)
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();

    if let Some(()) = handle_early_args(&args).await? {
        return Ok(());
    }

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
        setup::override_profile_config(&mut config, profile_name, &paths)?;
        let profile_config_path = paths.profile_home().join("config").join("config.json");
        if profile_config_path.is_file() {
            config.merge_profile_file(&profile_config_path)?;
        }
    }

    let startup_lang = config.language.as_deref().unwrap_or("en");
    i18n::init(startup_lang);

    if let Some(()) = handle_daemon_mode(&args, &config).await? {
        return Ok(());
    }

    if let Some(ref p) = worker_profile {
        println!("🤖 [tuner] Starting Telegram bot worker for '{}'...", p);
        telegram::run_bot(config, paths).await
    } else {
        run_master_mode(config).await
    }
}

// install_systemd_service moved to config module
