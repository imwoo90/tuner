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




#[tokio::main]
async fn main() -> Result<(), String> {
    i18n::init("ko");

    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/wimvm".to_string());
    let config_path = std::path::PathBuf::from(home)
        .join(".ductor")
        .join("config")
        .join("config.json");

    let config = config::CliConfig::load_from_file(&config_path)?;

    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
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


    println!("🤖 [우덕터] Loading config from: {:?}", config_path);
    println!("🤖 [우덕터] Starting Telegram bot...");
    telegram::run_bot(config).await?;
    
    Ok(())
}

fn install_systemd_service(config: &config::CliConfig) -> Result<(), String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/wimvm".to_string());
    let token = std::env::var("TELEGRAM_TOKEN")
        .unwrap_or_else(|_| config.telegram_token.clone());
        
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Failed to resolve current binary path: {}", e))?;
        
    let unit_content = format!(
        "[Unit]\n\
         Description=Tuner - Telegram Bot Daemon for Antigravity CLI\n\
         After=network.target\n\n\
         [Service]\n\
         Type=simple\n\
         ExecStart={}\n\
         Environment=\"TELEGRAM_TOKEN={}\"\n\
         Environment=\"HOME={}\"\n\
         Restart=always\n\
         RestartSec=10\n\n\
         [Install]\n\
         WantedBy=default.target\n",
        current_exe.to_string_lossy(),
        token,
        home
    );

    let systemd_dir = std::path::PathBuf::from(&home)
        .join(".config")
        .join("systemd")
        .join("user");
        
    std::fs::create_dir_all(&systemd_dir)
        .map_err(|e| format!("Failed to create systemd user directory: {}", e))?;
        
    let service_file = systemd_dir.join("tuner.service");
    std::fs::write(&service_file, unit_content)
        .map_err(|e| format!("Failed to write tuner.service: {}", e))?;

    println!("🤖 [우덕터] Tuner systemd user service installed successfully!");
    println!("📂 Service Path: {:?}", service_file);
    println!("💡 Run the following commands to enable and start Tuner:");
    println!("   systemctl --user daemon-reload");
    println!("   systemctl --user enable tuner");
    println!("   systemctl --user start tuner");
    println!("   systemctl --user status tuner");
    
    Ok(())
}
