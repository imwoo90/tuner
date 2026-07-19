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

pub async fn perform_upgrade(url: &str) -> Result<(), String> {
    let exe_path = std::env::current_exe()
        .map_err(|e| format!("Failed to resolve current binary path: {}", e))?;
    let parent_dir = exe_path.parent().ok_or("Failed to get executable directory")?;
    let tmp_path = parent_dir.join(format!(".tuner.tmp.{}", rand::random::<u32>()));

    let client = reqwest::Client::builder()
        .user_agent("tuner-updater")
        .build()
        .map_err(|e| e.to_string())?;

    let mut res = client.get(url)
        .send()
        .await
        .map_err(|e| format!("Failed to send download request: {}", e))?;

    if !res.status().is_success() {
        return Err(format!("Download request failed with status: {}", res.status()));
    }

    let mut tmp_file = tokio::fs::File::create(&tmp_path)
        .await
        .map_err(|e| format!("Failed to create temporary update file: {}", e))?;

    while let Some(chunk) = res.chunk().await.map_err(|e| format!("Error while downloading chunk: {}", e))? {
        tokio::io::AsyncWriteExt::write_all(&mut tmp_file, &chunk)
            .await
            .map_err(|e| format!("Failed to write binary chunk: {}", e))?;
    }
    tokio::io::AsyncWriteExt::flush(&mut tmp_file).await.map_err(|e| e.to_string())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Failed to make download executable: {}", e))?;
    }

    if let Err(e) = std::fs::rename(&tmp_path, &exe_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!("Failed to replace executable binary: {}", e));
    }

    Ok(())
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
        let asset = release.assets.iter().find(|a| a.name.contains("linux"))
            .ok_or("Could not find a valid release asset for Linux.")?;
        
        println!("Downloading update from {}...", asset.browser_download_url);
        perform_upgrade(&asset.browser_download_url).await?;
        println!("Upgrade complete! Please restart the tuner daemon.");
    } else {
        println!("Already up to date. (Current version: {})", current);
    }
    Ok(())
}
