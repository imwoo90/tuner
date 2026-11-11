//! # Self-Upgrade and Release Version Control
//!
//! ## Overview
//! Manages checking for GitHub release versions, evaluating system-wide idle gates
//! (foreground turns, subagents, background tasks, child processes, and cron jobs),
//! downloading binaries, and atomically swapping the executable on idle restarts.
//!
//! ## Submodules
//! - [`activity`] (`activity.rs`): Multi-layer active-work detector across warm CLI processes, `/proc` child trees, and cron runs.
//! - [`brain_inspector`] (`brain_inspector.rs`): Recursive Antigravity subagent and background-task transcript inspector.
//! - [`step_parser`] (`step_parser.rs`): Compact JSONL step parser and timer duration decoder.
//! - [`task_tracker`] (`task_tracker.rs`): Monotonic `step_index` task state tracker for timers, crons, and `manage_task`.
//!
//! ## Search Tags
//! #binary-upgrade, #github-releases, #idle-gate, #subagent-guard

pub mod activity;
pub mod brain_inspector;
pub mod step_parser;
pub mod task_tracker;

pub use activity::{
    active_background_runs, active_cron_runs, active_foreground_runs, begin_background_run,
    begin_cron_run, begin_foreground_run, evaluate_active_work, has_active_descendant_processes,
    has_active_descendants_in_table, has_active_descendants_in_table_with_worker,
    is_idle_upgrade_window, observe_activity_epoch, record_user_activity, user_idle_secs,
    ActiveWorkStatus, BackgroundRunGuard, CronRunGuard, ForegroundRunGuard,
};
#[cfg(test)]
pub(crate) use activity::{now_epoch_secs, set_last_activity_epoch_for_test};
pub use brain_inspector::{inspect_agy_brain_activity, inspect_agy_brain_activity_with_ack};

use serde::Deserialize;
use std::path::Path;

/// Returns `true` if any provider (`agy`, `claude`, `codex`), subagent, background task, OS child command, or cron job is active.
pub async fn has_any_active_work(cli: &crate::cli::antigravity::AntigravityCli) -> bool {
    activity::detect_any_active_work(cli).await
}

async fn run_auto_upgrade_tick(
    config: &crate::config::CliConfig,
    cli: &crate::cli::antigravity::AntigravityCli,
) {
    if std::env::var("TUNER_DISABLE_AUTO_UPGRADE").is_ok_and(|v| v == "1" || v == "true") {
        return;
    }
    let work = activity::evaluate_active_work(cli).await;
    if work.is_busy {
        if work.is_user_activity {
            record_user_activity();
        }
        return;
    }
    let tz_str = config.user_timezone.as_deref().unwrap_or("Asia/Seoul");
    let tz = tz_str.parse::<chrono_tz::Tz>().unwrap_or(chrono_tz::Asia::Seoul);
    let local_hour = chrono::Timelike::hour(&chrono::Utc::now().with_timezone(&tz));
    if !is_idle_upgrade_window(local_hour, user_idle_secs(), false) {
        return;
    }
    if let Ok(rel) = get_latest_release().await {
        let latest = rel.tag_name.trim_start_matches('v');
        if is_newer_version(env!("CARGO_PKG_VERSION"), latest)
            && let Some(asset) = rel.assets.iter().find(|a| a.name.ends_with(".tar.gz"))
            && is_idle_upgrade_window(local_hour, user_idle_secs(), has_any_active_work(cli).await)
            && perform_upgrade(&asset.browser_download_url).await.is_ok()
            && is_idle_upgrade_window(local_hour, user_idle_secs(), has_any_active_work(cli).await)
        {
            println!(
                "🤖 [tuner] Auto-upgraded to v{} during idle window. Restarting...",
                latest
            );
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            std::process::exit(42);
        }
    }
}

/// Spawns the background unattended auto-upgrade task that checks GitHub Releases during idle windows.
pub fn spawn_auto_upgrade_scheduler(
    config: std::sync::Arc<crate::config::CliConfig>,
    cli: std::sync::Arc<crate::cli::antigravity::AntigravityCli>,
) {
    record_user_activity();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15 * 60));
        interval.tick().await;
        loop {
            interval.tick().await;
            run_auto_upgrade_tick(&config, &cli).await;
        }
    });
}

#[derive(Deserialize, Clone, Debug)]
/// Represents `GithubAsset` (github asset).
pub struct GithubAsset {
    /// Name.
    pub name: String,
    /// Browser download url.
    pub browser_download_url: String,
}

#[derive(Deserialize, Clone, Debug)]
/// Represents `GithubRelease` (github release).
pub struct GithubRelease {
    /// Tag name.
    pub tag_name: String,
    /// Body.
    pub body: Option<String>,
    /// Assets.
    pub assets: Vec<GithubAsset>,
}

/// Executes `get_latest_release` (get latest release).
pub async fn get_latest_release() -> Result<GithubRelease, String> {
    let client = reqwest::Client::builder()
        .user_agent("tuner-updater")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let res = client
        .get("https://api.github.com/repos/imwoo90/tuner/releases/latest")
        .send()
        .await
        .map_err(|e| format!("Failed to query latest release: {}", e))?;

    if !res.status().is_success() {
        return Err(format!("GitHub API status error: {}", res.status()));
    }

    res.json()
        .await
        .map_err(|e| format!("Failed to parse release JSON: {}", e))
}

/// Executes `is_newer_version` (is newer version).
pub fn is_newer_version(current: &str, latest: &str) -> bool {
    let parse_parts = |s: &str| -> Vec<u32> {
        s.trim_start_matches('v')
            .split('.')
            .map(|p| p.parse::<u32>().unwrap_or(0))
            .collect()
    };
    let curr_parts = parse_parts(current);
    let late_parts = parse_parts(latest);
    for i in 0..std::cmp::max(curr_parts.len(), late_parts.len()) {
        let curr_val = curr_parts.get(i).copied().unwrap_or(0);
        let late_val = late_parts.get(i).copied().unwrap_or(0);
        if late_val > curr_val {
            return true;
        } else if curr_val > late_val {
            return false;
        }
    }
    false
}

async fn download_archive(url: &str, dest: &Path) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent("tuner-updater")
        .build()
        .map_err(|e| e.to_string())?;
    let mut res = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download req failed: {}", e))?;
    if !res.status().is_success() {
        return Err(format!("Download failed status: {}", res.status()));
    }
    let mut tmp_file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("Create temp archive failed: {}", e))?;
    while let Some(chunk) = res
        .chunk()
        .await
        .map_err(|e| format!("Download error: {}", e))?
    {
        tokio::io::AsyncWriteExt::write_all(&mut tmp_file, &chunk)
            .await
            .map_err(|e| format!("Write failed: {}", e))?;
    }
    tokio::io::AsyncWriteExt::flush(&mut tmp_file)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn unpack_archive(archive_path: &Path, dest_path: &Path) -> Result<(), String> {
    let archive_cloned = archive_path.to_path_buf();
    let dest_cloned = dest_path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let file = std::fs::File::open(&archive_cloned)
            .map_err(|e| format!("Open archive failed: {}", e))?;
        let tar = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(tar);
        std::fs::create_dir_all(&dest_cloned)
            .map_err(|e| format!("Create dir failed: {}", e))?;
        archive
            .unpack(&dest_cloned)
            .map_err(|e| format!("Unpack failed: {}", e))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("Join error: {}", e))?
}

/// Executes `perform_upgrade` (perform upgrade).
pub async fn perform_upgrade(url: &str) -> Result<(), String> {
    let exe_path =
        std::env::current_exe().map_err(|e| format!("Failed to get exe path: {}", e))?;
    let parent_dir = exe_path.parent().ok_or("No exe parent dir")?;
    let r = rand::random::<u32>();
    let tmp_archive = parent_dir.join(format!(".tuner.archive.{}", r));
    let tmp_unpack_dir = parent_dir.join(format!(".tuner.unpack.{}", r));

    if let Err(e) = download_archive(url, &tmp_archive).await {
        let _ = std::fs::remove_file(&tmp_archive);
        return Err(e);
    }

    if let Err(e) = unpack_archive(&tmp_archive, &tmp_unpack_dir).await {
        let _ = std::fs::remove_file(&tmp_archive);
        let _ = std::fs::remove_dir_all(&tmp_unpack_dir);
        return Err(e);
    }

    let ext_root = tmp_unpack_dir.join("tuner-linux-amd64");
    let new_binary = ext_root.join("tuner");
    let new_defaults = ext_root.join("_home_defaults");

    if !new_binary.is_file() {
        let _ = std::fs::remove_file(&tmp_archive);
        let _ = std::fs::remove_dir_all(&tmp_unpack_dir);
        return Err("Package missing tuner binary".to_string());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&new_binary, std::fs::Permissions::from_mode(0o755));
    }

    let bin_ok = std::fs::rename(&new_binary, &exe_path);
    if bin_ok.is_ok() && new_defaults.is_dir() {
        let target_defaults = parent_dir.join("_home_defaults");
        if target_defaults.exists() {
            let _ = std::fs::remove_dir_all(&target_defaults);
        }
        let _ = std::fs::rename(&new_defaults, &target_defaults);
    }

    let _ = std::fs::remove_file(&tmp_archive);
    let _ = std::fs::remove_dir_all(&tmp_unpack_dir);
    bin_ok.map_err(|e| format!("Replace binary failed: {}", e))
}

/// Executes `run_cli_upgrade` (run cli upgrade).
pub async fn run_cli_upgrade() -> Result<(), String> {
    println!("Checking for updates...");
    let release = get_latest_release().await?;
    let current = env!("CARGO_PKG_VERSION");
    let latest = release.tag_name.trim_start_matches('v');

    if is_newer_version(current, latest) {
        println!("New version available: {} (current: {})", latest, current);
        let asset = release
            .assets
            .iter()
            .find(|a| a.name.ends_with(".tar.gz"))
            .ok_or("Could not find a valid release package (.tar.gz) for Linux.")?;

        println!("Downloading update from {}...", asset.browser_download_url);
        perform_upgrade(&asset.browser_download_url).await?;
        println!("Upgrade complete! Please restart the tuner daemon.");
    } else {
        println!("Already up to date. (Current version: {})", current);
    }
    Ok(())
}
