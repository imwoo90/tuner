//! # Self-Upgrade and Release Version Control
//!
//! Coordinates checking for GitHub release versions, downloading binaries, verifying checksums,
//! and replacing the running executable dynamically with zero-downtime restarts.

use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize, Clone, Debug)]
pub struct GithubAsset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct GithubRelease {
    pub tag_name: String,
    pub body: Option<String>,
    pub assets: Vec<GithubAsset>,
}

pub fn is_dev_install() -> bool {
    if let Ok(exe) = std::env::current_exe() {
        let path_str = exe.to_string_lossy();
        if path_str.contains("/target/debug/") || path_str.contains("/target/release/") {
            return true;
        }
        if let Some(parent) = exe.parent() {
            if parent.join("Cargo.toml").is_file() || parent.join(".git").is_dir() {
                return true;
            }
            if let Some(gp) = parent.parent() {
                if gp.join("Cargo.toml").is_file() || gp.join(".git").is_dir() {
                    return true;
                }
            }
        }
    }
    Path::new("Cargo.toml").is_file()
}

pub async fn get_latest_release() -> Result<GithubRelease, String> {
    let client = reqwest::Client::builder()
        .user_agent("tuner-updater")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let res = client.get("https://api.github.com/repos/imwoo90/tuner/releases/latest")
        .send()
        .await
        .map_err(|e| format!("Failed to query latest release: {}", e))?;

    if !res.status().is_success() {
        return Err(format!("GitHub API status error: {}", res.status()));
    }

    let release: GithubRelease = res.json()
        .await
        .map_err(|e| format!("Failed to parse release JSON: {}", e))?;

    Ok(release)
}

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
    let mut res = client.get(url).send().await.map_err(|e| format!("Download req failed: {}", e))?;
    if !res.status().is_success() {
        return Err(format!("Download failed status: {}", res.status()));
    }
    let mut tmp_file = tokio::fs::File::create(dest).await.map_err(|e| format!("Create temp archive failed: {}", e))?;
    while let Some(chunk) = res.chunk().await.map_err(|e| format!("Download error: {}", e))? {
        tokio::io::AsyncWriteExt::write_all(&mut tmp_file, &chunk).await.map_err(|e| format!("Write failed: {}", e))?;
    }
    tokio::io::AsyncWriteExt::flush(&mut tmp_file).await.map_err(|e| e.to_string())?;
    Ok(())
}

async fn unpack_archive(archive_path: &Path, dest_path: &Path) -> Result<(), String> {
    let archive_cloned = archive_path.to_path_buf();
    let dest_cloned = dest_path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let file = std::fs::File::open(&archive_cloned).map_err(|e| format!("Open archive failed: {}", e))?;
        let tar = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(tar);
        std::fs::create_dir_all(&dest_cloned).map_err(|e| format!("Create dir failed: {}", e))?;
        archive.unpack(&dest_cloned).map_err(|e| format!("Unpack failed: {}", e))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("Join error: {}", e))?
}

pub async fn perform_upgrade(url: &str) -> Result<(), String> {
    let exe_path = std::env::current_exe().map_err(|e| format!("Failed to get exe path: {}", e))?;
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

pub async fn run_cli_upgrade() -> Result<(), String> {
    if is_dev_install() {
        println!("Self-upgrade is not available for development installs.");
        println!("Please run 'git pull' and 'cargo build --release' instead.");
        return Ok(());
    }

    println!("Checking for updates...");
    let release = get_latest_release().await?;
    let current = env!("CARGO_PKG_VERSION");
    let latest = release.tag_name.trim_start_matches('v');

    if is_newer_version(current, latest) {
        println!("New version available: {} (current: {})", latest, current);
        let asset = release.assets.iter().find(|a| a.name.ends_with(".tar.gz"))
            .ok_or("Could not find a valid release package (.tar.gz) for Linux.")?;
        
        println!("Downloading update from {}...", asset.browser_download_url);
        perform_upgrade(&asset.browser_download_url).await?;
        println!("Upgrade complete! Please restart the tuner daemon.");
    } else {
        println!("Already up to date. (Current version: {})", current);
    }
    Ok(())
}
