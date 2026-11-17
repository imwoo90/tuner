//! # Web Auth Persistence & Per-IP Lockout Helpers (`auth_store.rs`)
//!
//! ## Overview
//! Handles `flock`-guarded `0600` persistence for `dashboard_auth.json`, session construction,
//! token pruning, and sliding per-IP brute-force tracking (`5` failures -> `60s` `429` lockout).
//!
//! ## Search Tags
//! #auth-storage, #file-flock, #ip-rate-limit, #0600-permissions

use super::auth::{AuthSession, DashboardAuthFile};
use crate::security::vault_auth::proxy::set_mode_0600;
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::Path;
use std::time::{Duration, Instant};

const AUTH_FILE: &str = "dashboard_auth.json";
const LOCK_FILE: &str = ".dashboard_auth.lock";
pub(crate) const SESSION_TTL_SECS: u64 = 30 * 86_400;
const MAX_FAILS: u32 = 5;
const MAX_GLOBAL_FAILS: u32 = 15;
const LOCKOUT_SECS: u64 = 60;
const FAIL_WINDOW_SECS: u64 = 900;
const MAX_FAIL_MAP_ENTRIES: usize = 2048;

pub(crate) struct FileFlockGuard {
    _file: std::fs::File,
}

impl FileFlockGuard {
    pub(crate) fn acquire(vault_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(vault_dir).map_err(|e| e.to_string())?;
        let lock_path = vault_dir.join(LOCK_FILE);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| e.to_string())?;
        set_mode_0600(&lock_path);
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            nix::fcntl::flock(file.as_raw_fd(), nix::fcntl::FlockArg::LockExclusive)
                .map_err(|e| e.to_string())?;
        }
        Ok(Self { _file: file })
    }
}

pub(crate) fn load_auth_file(vault_dir: &Path) -> Result<DashboardAuthFile, String> {
    let path = vault_dir.join(AUTH_FILE);
    if !path.is_file() {
        return Ok(DashboardAuthFile::default());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    if raw.trim().is_empty() {
        return Ok(DashboardAuthFile::default());
    }
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

pub(crate) fn save_auth_file(vault_dir: &Path, data: &DashboardAuthFile) -> Result<(), String> {
    std::fs::create_dir_all(vault_dir).map_err(|e| e.to_string())?;
    let dst = vault_dir.join(AUTH_FILE);
    let tmp = vault_dir.join(".dashboard_auth.json.tmp");
    let raw = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, raw).map_err(|e| e.to_string())?;
    set_mode_0600(&tmp);
    std::fs::rename(&tmp, &dst).map_err(|e| e.to_string())?;
    set_mode_0600(&dst);
    Ok(())
}

pub(crate) fn prune_expired(data: &mut DashboardAuthFile, now: u64) {
    data.magic_tokens.retain(|_, v| v.expires_at_secs > now);
    data.sessions.retain(|_, v| v.expires_at_secs > now);
}

pub(crate) fn build_new_session(now: u64) -> AuthSession {
    AuthSession {
        sid: hex::encode(rand::random::<[u8; 32]>()),
        csrf_token: hex::encode(rand::random::<[u8; 32]>()),
        created_at_secs: now,
        last_seen_secs: now,
        expires_at_secs: now.saturating_add(SESSION_TTL_SECS),
    }
}

pub(crate) struct FailEntry {
    pub(crate) count: u32,
    pub(crate) last_fail: Instant,
    pub(crate) locked_until: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct LoginRateState {
    pub(crate) per_ip: HashMap<IpAddr, FailEntry>,
    pub(crate) global_fails: u32,
    pub(crate) global_locked_until: Option<Instant>,
}

pub(crate) fn check_fail_map(state: &mut LoginRateState, ip: IpAddr) -> Result<(), u64> {
    let now = Instant::now();
    if let Some(g_until) = state.global_locked_until {
        if g_until > now {
            return Err(g_until.saturating_duration_since(now).as_secs().max(1));
        }
        state.global_locked_until = None;
        state.global_fails = 0;
    }
    state.per_ip.retain(|_, v| {
        v.locked_until.is_some_and(|t| t > now)
            || now.saturating_duration_since(v.last_fail) < Duration::from_secs(FAIL_WINDOW_SECS)
    });
    if let Some(entry) = state.per_ip.get_mut(&ip)
        && let Some(until) = entry.locked_until
    {
        if until > now {
            return Err(until.saturating_duration_since(now).as_secs().max(1));
        }
        entry.locked_until = None;
        entry.count = 0;
    }
    Ok(())
}

pub(crate) fn record_fail_map(state: &mut LoginRateState, ip: IpAddr) {
    let now = Instant::now();
    state.global_fails = state.global_fails.saturating_add(1);
    if state.global_fails >= MAX_GLOBAL_FAILS {
        state.global_locked_until = Some(now + Duration::from_secs(LOCKOUT_SECS));
    }
    if state.per_ip.len() >= MAX_FAIL_MAP_ENTRIES && !state.per_ip.contains_key(&ip) {
        state.per_ip.retain(|_, v| v.locked_until.is_some_and(|t| t > now));
        if state.per_ip.len() >= MAX_FAIL_MAP_ENTRIES {
            return;
        }
    }
    let entry = state.per_ip.entry(ip).or_insert(FailEntry {
        count: 0,
        last_fail: now,
        locked_until: None,
    });
    entry.count = entry.count.saturating_add(1);
    entry.last_fail = now;
    if entry.count >= MAX_FAILS {
        entry.locked_until = Some(now + Duration::from_secs(LOCKOUT_SECS));
    }
}
