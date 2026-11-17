//! # Web Dashboard Authentication, PIN Hashing & Brute-Force Guard (`auth.rs`)
//!
//! ## Overview
//! Manages authentication operations over persistent `0600` state (`dashboard_auth.json`):
//! single-use 256-bit Magic Links (`10m` TTL), `PBKDF2_HMAC_SHA256` (`100,000` rounds) PINs,
//! 30-day sliding cookies with CSRF tokens, and per-IP `429` lockouts.
//!
//! ## Search Tags
//! #web-auth, #magic-link, #pbkdf2-pin, #csrf-session, #rate-limiter

use super::auth_store::{
    build_new_session, check_fail_map, load_auth_file, prune_expired, record_fail_map,
    save_auth_file, FileFlockGuard, LoginRateState, SESSION_TTL_SECS,
};
use crate::security::vault::current_unix_secs;
use crate::security::vault_auth::proxy::{constant_time_eq, is_strict_hex_token};
use ring::pbkdf2;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::path::PathBuf;
use tokio::sync::Mutex;

const MAGIC_TTL_SECS: u64 = 600;
const SLIDING_WRITE_INTERVAL_SECS: u64 = 60;
const PBKDF2_ROUNDS: u32 = 100_000;

fn pbkdf2_iterations() -> NonZeroU32 {
    match NonZeroU32::new(PBKDF2_ROUNDS) {
        Some(n) => n,
        None => NonZeroU32::MIN,
    }
}

/// Stored PBKDF2-HMAC-SHA256 hash record for the owner PIN/passphrase.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PinRecord {
    /// Random 16-byte salt encoded as lowercase hex.
    pub salt_hex: String,
    /// Derived 32-byte key encoded as lowercase hex.
    pub hash_hex: String,
    /// Iteration count (`100_000`).
    pub iterations: u32,
}

/// Single-use Magic Link token metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MagicTokenRecord {
    /// Creation Unix timestamp in seconds.
    pub created_at_secs: u64,
    /// Expiration Unix timestamp in seconds.
    pub expires_at_secs: u64,
}

/// Active authenticated browser session persisted to `dashboard_auth.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthSession {
    /// 256-bit hex session cookie value (`tuner_dash_sid`).
    pub sid: String,
    /// Associated 256-bit hex CSRF token (`x-tuner-csrf`).
    pub csrf_token: String,
    /// Creation Unix timestamp in seconds.
    pub created_at_secs: u64,
    /// Last validated Unix timestamp in seconds.
    pub last_seen_secs: u64,
    /// Sliding expiration Unix timestamp in seconds.
    pub expires_at_secs: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct DashboardAuthFile {
    #[serde(default)]
    pub(crate) pin_hash: Option<PinRecord>,
    #[serde(default)]
    pub(crate) magic_tokens: HashMap<String, MagicTokenRecord>,
    #[serde(default)]
    pub(crate) sessions: HashMap<String, AuthSession>,
}

/// Thread-safe, cross-process `0600` authentication store for the Web Dashboard.
pub struct WebAuthStore {
    vault_dir: PathBuf,
    io_lock: Mutex<()>,
    fail_map: Mutex<LoginRateState>,
}

impl WebAuthStore {
    /// Creates an authentication store bound to `<vault_dir>/dashboard_auth.json`.
    pub fn new(vault_dir: PathBuf) -> Self {
        Self {
            vault_dir,
            io_lock: Mutex::new(()),
            fail_map: Mutex::new(LoginRateState::default()),
        }
    }

    /// Returns whether an owner PIN/passphrase is currently configured.
    pub async fn has_pin(&self) -> bool {
        let _g = self.io_lock.lock().await;
        load_auth_file(&self.vault_dir).ok().and_then(|d| d.pin_hash).is_some()
    }

    /// Sets or updates the owner PIN/passphrase using `PBKDF2_HMAC_SHA256` (`100,000` rounds).
    pub async fn set_pin(&self, pin: &str) -> Result<(), String> {
        if pin.len() < 4 || pin.len() > 256 {
            return Err("PIN/passphrase must be between 4 and 256 characters".to_string());
        }
        let salt: [u8; 16] = rand::random();
        let mut derived = [0u8; 32];
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            pbkdf2_iterations(),
            &salt,
            pin.as_bytes(),
            &mut derived,
        );
        let rec = PinRecord {
            salt_hex: hex::encode(salt),
            hash_hex: hex::encode(derived),
            iterations: PBKDF2_ROUNDS,
        };
        let _g = self.io_lock.lock().await;
        let _fl = FileFlockGuard::acquire(&self.vault_dir)?;
        let mut data = load_auth_file(&self.vault_dir)?;
        data.pin_hash = Some(rec);
        save_auth_file(&self.vault_dir, &data)
    }

    /// Verifies `pin` in constant time via `ring::pbkdf2::verify`.
    pub async fn verify_pin(&self, pin: &str) -> Result<bool, String> {
        let _g = self.io_lock.lock().await;
        let data = load_auth_file(&self.vault_dir)?;
        let Some(rec) = data.pin_hash else {
            return Ok(false);
        };
        let salt = hex::decode(&rec.salt_hex).map_err(|e| e.to_string())?;
        let hash = hex::decode(&rec.hash_hex).map_err(|e| e.to_string())?;
        let rounds = NonZeroU32::new(rec.iterations).unwrap_or_else(pbkdf2_iterations);
        Ok(pbkdf2::verify(pbkdf2::PBKDF2_HMAC_SHA256, rounds, &salt, pin.as_bytes(), &hash).is_ok())
    }

    /// Issues a single-use 256-bit (64-hex) Magic Link token with 10-minute TTL.
    pub async fn issue_magic_token(&self) -> Result<String, String> {
        let _g = self.io_lock.lock().await;
        let _fl = FileFlockGuard::acquire(&self.vault_dir)?;
        let now = current_unix_secs();
        let mut data = load_auth_file(&self.vault_dir)?;
        prune_expired(&mut data, now);
        let tok = hex::encode(rand::random::<[u8; 32]>());
        data.magic_tokens.insert(
            tok.clone(),
            MagicTokenRecord { created_at_secs: now, expires_at_secs: now.saturating_add(MAGIC_TTL_SECS) },
        );
        save_auth_file(&self.vault_dir, &data)?;
        Ok(tok)
    }

    /// Atomically validates and consumes a Magic Link token, returning a new [`AuthSession`].
    pub async fn consume_magic_token(&self, token: &str) -> Result<Option<AuthSession>, String> {
        if !is_strict_hex_token(token) {
            return Ok(None);
        }
        let _g = self.io_lock.lock().await;
        let _fl = FileFlockGuard::acquire(&self.vault_dir)?;
        let now = current_unix_secs();
        let mut data = load_auth_file(&self.vault_dir)?;
        prune_expired(&mut data, now);
        let key = data
            .magic_tokens
            .keys()
            .find(|k| constant_time_eq(k.as_bytes(), token.as_bytes()))
            .cloned();
        let Some(key) = key else {
            return Ok(None);
        };
        data.magic_tokens.remove(&key);
        let sess = build_new_session(now);
        data.sessions.insert(sess.sid.clone(), sess.clone());
        save_auth_file(&self.vault_dir, &data)?;
        {
            let mut rate = self.fail_map.lock().await;
            rate.global_fails = 0;
            rate.global_locked_until = None;
        }
        Ok(Some(sess))
    }

    /// Creates and persists a new authenticated browser session after PIN login.
    pub async fn create_session(&self) -> Result<AuthSession, String> {
        let _g = self.io_lock.lock().await;
        let _fl = FileFlockGuard::acquire(&self.vault_dir)?;
        let now = current_unix_secs();
        let mut data = load_auth_file(&self.vault_dir)?;
        prune_expired(&mut data, now);
        let sess = build_new_session(now);
        data.sessions.insert(sess.sid.clone(), sess.clone());
        save_auth_file(&self.vault_dir, &data)?;
        Ok(sess)
    }

    /// Validates a `tuner_dash_sid` cookie in constant time and updates sliding expiry.
    pub async fn verify_session(&self, sid: &str) -> Option<AuthSession> {
        if !is_strict_hex_token(sid) {
            return None;
        }
        let _g = self.io_lock.lock().await;
        let now = current_unix_secs();
        let data = load_auth_file(&self.vault_dir).ok()?;
        let matched = data
            .sessions
            .iter()
            .find(|(k, v)| v.expires_at_secs > now && constant_time_eq(k.as_bytes(), sid.as_bytes()))
            .map(|(k, v)| (k.clone(), v.clone()));
        let (key, mut sess) = matched?;
        if now.saturating_sub(sess.last_seen_secs) >= SLIDING_WRITE_INTERVAL_SECS
            && let Ok(_fl) = FileFlockGuard::acquire(&self.vault_dir)
            && let Ok(mut fresh) = load_auth_file(&self.vault_dir)
        {
            prune_expired(&mut fresh, now);
            sess.last_seen_secs = now;
            sess.expires_at_secs = now.saturating_add(SESSION_TTL_SECS);
            if let Some(slot) = fresh.sessions.get_mut(&key) {
                *slot = sess.clone();
                let _ = save_auth_file(&self.vault_dir, &fresh);
            }
        }
        Some(sess)
    }

    /// Revokes a single session by cookie ID.
    pub async fn revoke_session(&self, sid: &str) -> Result<bool, String> {
        if !is_strict_hex_token(sid) {
            return Ok(false);
        }
        let _g = self.io_lock.lock().await;
        let _fl = FileFlockGuard::acquire(&self.vault_dir)?;
        let mut data = load_auth_file(&self.vault_dir)?;
        let target = data
            .sessions
            .keys()
            .find(|k| constant_time_eq(k.as_bytes(), sid.as_bytes()))
            .cloned();
        let removed = target.is_some_and(|k| data.sessions.remove(&k).is_some());
        if removed {
            save_auth_file(&self.vault_dir, &data)?;
        }
        Ok(removed)
    }

    /// Immediately purges all active browser sessions and unconsumed magic tokens.
    pub async fn revoke_all(&self) -> Result<(), String> {
        let _g = self.io_lock.lock().await;
        let _fl = FileFlockGuard::acquire(&self.vault_dir)?;
        let mut data = load_auth_file(&self.vault_dir)?;
        data.sessions.clear();
        data.magic_tokens.clear();
        save_auth_file(&self.vault_dir, &data)
    }

    /// Checks whether `ip` is currently rate-limited (`429`), returning `Err(retry_after_secs)`.
    pub async fn check_login_rate_limit(&self, ip: IpAddr) -> Result<(), u64> {
        let mut map = self.fail_map.lock().await;
        check_fail_map(&mut map, ip)
    }

    /// Records a failed login attempt for `ip`, locking out after 5 consecutive failures.
    pub async fn record_login_failure(&self, ip: IpAddr) {
        let mut map = self.fail_map.lock().await;
        record_fail_map(&mut map, ip);
    }

    /// Clears consecutive failure tracking for `ip` after a successful login.
    pub async fn clear_login_failures(&self, ip: IpAddr) {
        let mut rate = self.fail_map.lock().await;
        rate.per_ip.remove(&ip);
        rate.global_fails = 0;
        rate.global_locked_until = None;
    }
}
