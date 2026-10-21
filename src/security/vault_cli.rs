//! # Zero-Knowledge Credential Vault CLI Runner
//!
//! ## Overview
//! Executes `--vault` CLI subcommands (`list`, `exec`, `session`) for AI agents and scripts.
//! Ensures plaintext passwords never enter LLM transcripts by injecting credentials strictly
//! through child process environment variables and scrubbing output streams.
//!
//! ## Search Tags
//! #vault-cli, #zero-knowledge-exec, #secret-redaction

use super::vault::{current_unix_secs, VaultStore};
use crate::workspace::paths::resolve_paths;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Ephemeral Web Vault session record persisted across worker restarts (15-minute TTL).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct VaultWebSession {
    /// Creation timestamp in Unix seconds.
    pub created_at_secs: u64,
    /// Target profile name.
    pub profile: String,
    /// Target profile vault directory.
    pub vault_dir: PathBuf,
}

/// Loads persisted `vault_sessions.json` from `vault_dir`.
pub fn load_persisted_sessions(vault_dir: &Path) -> HashMap<String, VaultWebSession> {
    let f = vault_dir.join("vault_sessions.json");
    std::fs::read_to_string(f)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Saves `vault_sessions.json` into `vault_dir`.
pub fn save_persisted_sessions(vault_dir: &Path, map: &HashMap<String, VaultWebSession>) {
    let _ = std::fs::create_dir_all(vault_dir);
    if let Ok(s) = serde_json::to_string(map) {
        let _ = std::fs::write(vault_dir.join("vault_sessions.json"), s);
    }
}

/// Detects the active profile name from CLI args, `TUNER_PROFILE`, or current working directory.
pub fn resolve_cli_profile(args: &[String]) -> String {
    if let Some(pos) = args.iter().position(|a| a == "--profile")
        && let Some(val) = args.get(pos + 1)
        && !val.trim().is_empty()
    {
        return val.trim().to_string();
    }
    if let Ok(env_prof) = std::env::var("TUNER_PROFILE")
        && !env_prof.trim().is_empty()
    {
        return env_prof.trim().to_string();
    }
    if let Ok(cwd) = std::env::current_dir()
        && let Some(detected) = extract_profile_from_path(&cwd)
    {
        return detected;
    }
    "default".to_string()
}

/// Extracts `<profile>` if `path` contains `.../profiles/<profile>/...`.
pub fn extract_profile_from_path(path: &Path) -> Option<String> {
    let comps: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    for i in 0..comps.len() {
        if comps[i] == "profiles"
            && let Some(next) = comps.get(i + 1)
            && !next.is_empty()
        {
            return Some(next.clone());
        }
    }
    None
}

/// Redacts sensitive secret strings from command output text.
pub fn redact_secrets(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_string();
    for secret in secrets {
        let trimmed = secret.trim();
        if trimmed.len() >= 3 {
            out = out.replace(trimmed, "***REDACTED***");
        }
    }
    out
}

fn open_profile_store(profile: &str) -> VaultStore {
    let paths = resolve_paths(None, None, None, Some(profile.to_string()));
    VaultStore::new(paths.vault_dir())
}

fn print_vault_help() {
    println!(
        "Usage:
  tuner --vault list [--profile <name>]
  tuner --vault exec <site_key> [--profile <name>] -- <command> [args...]

Environment variables injected during `exec`:
  TUNER_VAULT_SITE   Normalized site key (e.g. github)
  TUNER_VAULT_TITLE  Service display name
  TUNER_VAULT_URL    Login or service URL
  TUNER_VAULT_USER   Account username / email
  TUNER_VAULT_PASS   Account password (auto-redacted from stdout/stderr)"
    );
}

fn handle_vault_list(profile: &str) -> Result<(), String> {
    let store = open_profile_store(profile);
    let masked = store.list_masked()?;
    if masked.is_empty() {
        println!("🔐 [Vault: {}] No site credentials registered yet. Run `/vault` in Telegram to add credentials.", profile);
        return Ok(());
    }
    println!("🔐 [Vault: {}] {} registered site(s):", profile, masked.len());
    for item in &masked {
        let url_str = if item.url.is_empty() { "-" } else { &item.url };
        println!(
            "  • [{}] {} ({}) | user: {}",
            item.site_key, item.title, url_str, item.masked_username
        );
    }
    Ok(())
}

fn handle_vault_exec(profile: &str, site_key: &str, cmd_args: &[String]) -> Result<(), String> {
    if cmd_args.is_empty() {
        return Err("Missing command after `--`. Usage: tuner --vault exec <site_key> -- <command> [args...]".to_string());
    }
    let store = open_profile_store(profile);
    let entry = store
        .get_entry(site_key)?
        .ok_or_else(|| format!("Site '{}' not found in profile '{}' vault", site_key, profile))?;

    let mut cmd = std::process::Command::new(&cmd_args[0]);
    if cmd_args.len() > 1 {
        cmd.args(&cmd_args[1..]);
    }
    cmd.env("TUNER_VAULT_SITE", &entry.site_key)
        .env("TUNER_VAULT_TITLE", &entry.title)
        .env("TUNER_VAULT_URL", &entry.url)
        .env("TUNER_VAULT_USER", &entry.username)
        .env("TUNER_VAULT_PASS", &entry.password);

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to execute '{}': {}", cmd_args[0], e))?;

    let secrets = [entry.password.as_str()];
    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let stderr_str = String::from_utf8_lossy(&output.stderr);
    let clean_out = redact_secrets(&stdout_str, &secrets);
    let clean_err = redact_secrets(&stderr_str, &secrets);

    if !clean_out.is_empty() {
        print!("{}", clean_out);
    }
    if !clean_err.is_empty() {
        eprint!("{}", clean_err);
    }
    if !output.status.success() {
        let code = output.status.code().unwrap_or(1);
        return Err(format!("Command exited with code {}", code));
    }
    Ok(())
}

fn handle_vault_session(profile: &str) -> Result<(), String> {
    let store = open_profile_store(profile);
    let dir = store.dir().to_path_buf();
    let tok = hex::encode(rand::random::<[u8; 16]>());
    let mut map = load_persisted_sessions(&dir);
    map.insert(
        tok.clone(),
        VaultWebSession {
            created_at_secs: current_unix_secs(),
            profile: profile.to_string(),
            vault_dir: dir.clone(),
        },
    );
    save_persisted_sessions(&dir, &map);
    println!("{}", tok);
    Ok(())
}

/// Entrypoint for `tuner --vault ...` CLI invocation.
pub fn run_vault_cli(args: &[String]) -> Result<(), String> {
    let vault_pos = match args.iter().position(|a| a == "--vault") {
        Some(p) => p,
        None => return Ok(()),
    };
    let profile = resolve_cli_profile(args);
    let sub = args.get(vault_pos + 1).map(String::as_str).unwrap_or("help");

    match sub {
        "list" | "ls" => handle_vault_list(&profile),
        "session" | "token" => handle_vault_session(&profile),
        "exec" | "run" => {
            let site = args
                .get(vault_pos + 2)
                .ok_or_else(|| "Missing <site_key>. Usage: tuner --vault exec <site_key> -- <cmd>".to_string())?;
            let sep_pos = args
                .iter()
                .position(|a| a == "--")
                .ok_or_else(|| "Missing `--` separator before command".to_string())?;
            let cmd_slice = if sep_pos + 1 < args.len() {
                &args[sep_pos + 1..]
            } else {
                &[]
            };
            handle_vault_exec(&profile, site, cmd_slice)
        }
        _ => {
            print_vault_help();
            Ok(())
        }
    }
}
