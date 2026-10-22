//! # Encrypted Per-Profile Credential Vault Store
//!
//! ## Overview
//! Provides authenticated encryption (`XSalsa20-Poly1305` via `crypto_box::SalsaBox`) for
//! per-profile site login credentials (`vault.enc` + `0600` `vault.key`), and zero-knowledge
//! masked metadata views so LLM contexts never ingest raw passwords.
//!
//! ## Search Tags
//! #credential-vault, #aead-encryption, #profile-isolation, #zero-knowledge

use base64::Engine;
use crypto_box::aead::Aead;
use crypto_box::{SalsaBox, SecretKey};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Stored login credential record for a single site or service.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultEntry {
    /// Unique normalized identifier (e.g., `jlcpcb`, `github`).
    pub site_key: String,
    /// Human-readable service title (e.g., `JLCPCB`).
    pub title: String,
    /// Login or service URL.
    #[serde(default)]
    pub url: String,
    /// Account username, ID, or email.
    pub username: String,
    /// Account password (encrypted at rest).
    pub password: String,
    /// Last modification timestamp in Unix seconds.
    #[serde(default)]
    pub updated_at_secs: u64,
}

/// Zero-knowledge metadata summary safe for Telegram chat and LLM prompts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultEntryMasked {
    /// Unique normalized identifier.
    pub site_key: String,
    /// Human-readable service title.
    pub title: String,
    /// Login or service URL.
    pub url: String,
    /// Partially masked username (e.g., `im***@gmail.com`).
    pub masked_username: String,
    /// Whether a non-empty password is stored.
    pub has_password: bool,
    /// Last modification timestamp in Unix seconds.
    pub updated_at_secs: u64,
}

/// Returns current Unix timestamp in seconds.
pub fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Normalizes a raw site key into a clean lowercase slug (alphanumeric/Hangul, `-`, `_`, `.`).
pub fn normalize_site_key(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.trim().chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_alphanumeric() || lower == '-' || lower == '_' || lower == '.' {
            out.push(lower);
        } else if (lower.is_whitespace() || lower == '/') && !out.ends_with('-') && !out.is_empty()
        {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Masks a username so it can be safely displayed in chat or agent logs.
pub fn mask_username(username: &str) -> String {
    let trimmed = username.trim();
    if trimmed.is_empty() {
        return "(none)".to_string();
    }
    if let Some((local, domain)) = trimmed.split_once('@') {
        let vis: String = local.chars().take(2).collect();
        return format!("{}***@{}", vis, domain);
    }
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 2 {
        return format!("{}***", chars[0]);
    }
    let vis: String = chars.into_iter().take(3).collect();
    format!("{}***", vis)
}

impl VaultEntry {
    /// Converts this entry into a masked summary safe for chat and LLM tools.
    pub fn to_masked(&self) -> VaultEntryMasked {
        VaultEntryMasked {
            site_key: self.site_key.clone(),
            title: self.title.clone(),
            url: self.url.clone(),
            masked_username: mask_username(&self.username),
            has_password: !self.password.is_empty(),
            updated_at_secs: self.updated_at_secs,
        }
    }
}

/// Manages encrypted credential storage inside a profile's `vault` directory.
#[derive(Clone, Debug)]
pub struct VaultStore {
    vault_dir: PathBuf,
}

impl VaultStore {
    /// Creates a store bound to `vault_dir` (e.g. `~/.tuner/profiles/default/vault`).
    pub fn new(vault_dir: impl Into<PathBuf>) -> Self {
        Self {
            vault_dir: vault_dir.into(),
        }
    }

    /// Returns the underlying directory path.
    pub fn dir(&self) -> &Path {
        &self.vault_dir
    }

    fn key_path(&self) -> PathBuf {
        self.vault_dir.join("vault.key")
    }

    fn enc_path(&self) -> PathBuf {
        self.vault_dir.join("vault.enc")
    }

    fn load_or_create_box(&self) -> Result<SalsaBox, String> {
        std::fs::create_dir_all(&self.vault_dir).map_err(|e| e.to_string())?;
        let key_file = self.key_path();
        let secret_key = if key_file.is_file() {
            let raw_hex = std::fs::read_to_string(&key_file).map_err(|e| e.to_string())?;
            let decoded = hex::decode(raw_hex.trim()).map_err(|e| e.to_string())?;
            if decoded.len() != 32 {
                return Err("Invalid vault.key length".to_string());
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&decoded);
            SecretKey::from(arr)
        } else {
            let sk = SecretKey::generate(&mut OsRng);
            let encoded = hex::encode(sk.to_bytes());
            std::fs::write(&key_file, encoded).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600));
            }
            sk
        };
        let pub_key = secret_key.public_key();
        Ok(SalsaBox::new(&pub_key, &secret_key))
    }

    /// Decrypts and loads all entries from `vault.enc`. Returns an empty vector if not yet created.
    pub fn load_entries(&self) -> Result<Vec<VaultEntry>, String> {
        let enc_file = self.enc_path();
        if !enc_file.is_file() {
            return Ok(Vec::new());
        }
        let b64 = std::fs::read_to_string(&enc_file).map_err(|e| e.to_string())?;
        if b64.trim().is_empty() {
            return Ok(Vec::new());
        }
        let raw = base64::prelude::BASE64_STANDARD
            .decode(b64.trim())
            .map_err(|e| e.to_string())?;
        if raw.len() < 24 {
            return Err("Encrypted vault payload too short".to_string());
        }
        let (nonce_slice, cipher) = raw.split_at(24);
        let mut nonce_arr = [0u8; 24];
        nonce_arr.copy_from_slice(nonce_slice);
        let nonce = crypto_box::Nonce::from(nonce_arr);
        let sbox = self.load_or_create_box()?;
        let plain = sbox
            .decrypt(&nonce, cipher)
            .map_err(|_| "Vault decryption failed (key mismatch or corrupted data)".to_string())?;
        serde_json::from_slice(&plain).map_err(|e| e.to_string())
    }

    /// Encrypts and atomically writes `entries` to `vault.enc`.
    pub fn save_entries(&self, entries: &[VaultEntry]) -> Result<(), String> {
        let sbox = self.load_or_create_box()?;
        let plain = serde_json::to_vec(entries).map_err(|e| e.to_string())?;
        let mut nonce_arr = [0u8; 24];
        OsRng.fill_bytes(&mut nonce_arr);
        let nonce = crypto_box::Nonce::from(nonce_arr);
        let cipher = sbox
            .encrypt(&nonce, plain.as_slice())
            .map_err(|_| "Vault encryption failed".to_string())?;
        let mut packet = nonce_arr.to_vec();
        packet.extend(cipher);
        let b64 = base64::prelude::BASE64_STANDARD.encode(packet);

        let tmp_file = self.vault_dir.join("vault.enc.tmp");
        std::fs::write(&tmp_file, b64).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp_file, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp_file, self.enc_path()).map_err(|e| e.to_string())
    }

    /// Inserts or updates a credential entry, sorting entries alphabetically by `site_key`.
    pub fn upsert_entry(&self, mut entry: VaultEntry) -> Result<VaultEntry, String> {
        let key = normalize_site_key(if entry.site_key.trim().is_empty() {
            &entry.title
        } else {
            &entry.site_key
        });
        if key.is_empty() {
            return Err("site_key cannot be empty".to_string());
        }
        entry.site_key = key.clone();
        if entry.title.trim().is_empty() {
            entry.title = key.clone();
        }
        entry.updated_at_secs = current_unix_secs();

        let mut list = self.load_entries()?;
        if let Some(existing) = list.iter_mut().find(|e| e.site_key == key) {
            if entry.password.is_empty() {
                entry.password = existing.password.clone();
            }
            *existing = entry.clone();
        } else {
            list.push(entry.clone());
        }
        list.sort_by(|a, b| a.site_key.cmp(&b.site_key));
        self.save_entries(&list)?;
        Ok(entry)
    }

    /// Deletes an entry by `site_key`. Returns `true` if an entry was removed.
    pub fn delete_entry(&self, site_key: &str) -> Result<bool, String> {
        let norm = normalize_site_key(site_key);
        let mut list = self.load_entries()?;
        let orig_len = list.len();
        list.retain(|e| e.site_key != norm);
        if list.len() == orig_len {
            return Ok(false);
        }
        self.save_entries(&list)?;
        Ok(true)
    }

    /// Retrieves a single decrypted entry by `site_key`.
    pub fn get_entry(&self, site_key: &str) -> Result<Option<VaultEntry>, String> {
        let norm = normalize_site_key(site_key);
        let list = self.load_entries()?;
        Ok(list.into_iter().find(|e| e.site_key == norm))
    }

    /// Lists all entries in masked zero-knowledge form.
    pub fn list_masked(&self) -> Result<Vec<VaultEntryMasked>, String> {
        let list = self.load_entries()?;
        Ok(list.iter().map(VaultEntry::to_masked).collect())
    }
}
