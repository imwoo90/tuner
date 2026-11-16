//! # CLI Option Parser & Vault Target Resolver (`options.rs`)
//!
//! ## Overview
//! Parses `tuner --vault auth` CLI arguments (`<site_key>`, `--url`, `--ttl`, `--state-name`,
//! `--profile`, `--no-tunnel`), maps vendor domains to canonical storage slugs, and resolves
//! persistent profile directories alongside decrypted [`crate::security::vault::VaultEntry`] records.
//!
//! ## Search Tags
//! #vault-auth-options, #storage-slug, #target-resolver

use super::cdp_state::validate_http_url;
use crate::security::vault::{normalize_site_key, VaultEntry, VaultStore};
use std::path::{Path, PathBuf};

/// Default HITL remote browser lifetime in seconds (`30` minutes).
pub const DEFAULT_AUTH_TTL_SECS: u64 = 1800;

/// Parsed CLI options for `tuner --vault auth`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultAuthOptions {
    /// Requested Vault site key or alias (e.g. `aliexpress.com`, `https-jlcpcb.com`, `google`).
    pub site_key: String,
    /// Optional explicit initial HTTP/HTTPS URL override.
    pub start_url: Option<String>,
    /// Optional explicit storage slug override (`chromium_<slug>_ctx` and `<slug>_storage_state.json`).
    pub state_name: Option<String>,
    /// Session lifetime in seconds (`10..=7200`).
    pub ttl_secs: u64,
    /// Whether to skip external Cloudflare tunneling and bind loopback only (`127.0.0.1`).
    pub no_tunnel: bool,
}

/// Resolved filesystem paths, initial URL, and optional Vault credentials for an auth session.
#[derive(Clone, Debug)]
pub struct ResolvedAuthTarget {
    /// Canonical storage slug (e.g. `aliexpress`, `jlcpcb`, `lcsc`, `devicemart`, `google`).
    pub slug: String,
    /// Persistent Chrome profile directory (`<vault_dir>/chromium_<slug>_ctx`).
    pub context_dir: PathBuf,
    /// Exported Playwright state path (`<vault_dir>/<slug>_storage_state.json`).
    pub state_path: PathBuf,
    /// Validated `http://` or `https://` start URL.
    pub start_url: String,
    /// Matching decrypted `VaultEntry`, if registered in `VaultStore`.
    pub entry: Option<VaultEntry>,
}

/// Maps common Vault site keys/domains to canonical storage state slugs shared across skills.
///
/// # Examples
/// ```
/// use tuner::security::vault_auth::canonical_storage_slug;
/// assert_eq!(canonical_storage_slug("https-jlcpcb.com"), "jlcpcb");
/// assert_eq!(canonical_storage_slug("https-www.lcsc.com"), "lcsc");
/// assert_eq!(canonical_storage_slug("aliexpress.com"), "aliexpress");
/// assert_eq!(canonical_storage_slug("devicemart.co.kr"), "devicemart");
/// assert_eq!(canonical_storage_slug("wimzzang@gmail.com"), "google");
/// ```
pub fn canonical_storage_slug(raw_site_key: &str) -> String {
    let norm = normalize_site_key(raw_site_key);
    let stripped = norm
        .trim_start_matches("https-")
        .trim_start_matches("http-")
        .trim_start_matches("www.");
    if stripped.starts_with("jlcpcb.") || stripped == "jlcpcb" || stripped.starts_with("jlc3dp.") {
        return "jlcpcb".to_string();
    }
    if stripped.starts_with("lcsc.") || stripped == "lcsc" {
        return "lcsc".to_string();
    }
    if stripped.starts_with("aliexpress.") || stripped == "aliexpress" {
        return "aliexpress".to_string();
    }
    if stripped.starts_with("devicemart.") || stripped == "devicemart" {
        return "devicemart".to_string();
    }
    if stripped == "google"
        || stripped.starts_with("google.")
        || stripped.starts_with("accounts.google.")
        || raw_site_key.trim().eq_ignore_ascii_case("wimzzang@gmail.com")
    {
        return "google".to_string();
    }
    if stripped == "holtlab" || raw_site_key.trim().eq_ignore_ascii_case("holtlab.io@gmail.com") {
        return "holtlab".to_string();
    }
    let clean: String = stripped
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    let trimmed = clean.trim_matches('_');
    if trimmed.is_empty() {
        "default_site".to_string()
    } else {
        trimmed.to_string()
    }
}

fn default_url_for_slug(slug: &str, raw_key: &str) -> String {
    match slug {
        "jlcpcb" => "https://jlcpcb.com/user-center/fileManager".to_string(),
        "lcsc" => "https://www.lcsc.com/".to_string(),
        "aliexpress" => "https://www.aliexpress.com/p/ug-login-page/login.html".to_string(),
        "devicemart" => "https://www.devicemart.co.kr/member/login".to_string(),
        "google" | "holtlab" => "https://accounts.google.com/".to_string(),
        _ => {
            let trimmed = raw_key
                .trim()
                .trim_start_matches("https-")
                .trim_start_matches("http-");
            if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                trimmed.to_string()
            } else if trimmed.contains('.') && !trimmed.contains('@') {
                format!("https://{trimmed}")
            } else {
                "https://www.google.com/".to_string()
            }
        }
    }
}

fn consume_option_flag(
    args: &[String],
    idx: usize,
    opts: &mut VaultAuthOptions,
) -> Result<usize, String> {
    let flag = args[idx].as_str();
    match flag {
        "--profile" => {
            let val = args
                .get(idx + 1)
                .ok_or_else(|| "Missing profile name after `--profile`".to_string())?;
            if val.trim().is_empty() || val.starts_with('-') {
                return Err("Invalid profile name after `--profile`".to_string());
            }
            Ok(2)
        }
        "--no-tunnel" => {
            opts.no_tunnel = true;
            Ok(1)
        }
        "--url" => {
            let val = args
                .get(idx + 1)
                .ok_or_else(|| "Missing URL value after `--url`".to_string())?;
            opts.start_url = Some(validate_http_url(val)?);
            Ok(2)
        }
        "--state-name" => {
            let val = args
                .get(idx + 1)
                .ok_or_else(|| "Missing name after `--state-name`".to_string())?;
            opts.state_name = Some(canonical_storage_slug(val));
            Ok(2)
        }
        "--ttl" => {
            let val = args
                .get(idx + 1)
                .ok_or_else(|| "Missing seconds after `--ttl`".to_string())?;
            let parsed = val
                .parse::<u64>()
                .map_err(|_| format!("Invalid `--ttl` integer: '{val}'"))?;
            opts.ttl_secs = parsed.clamp(10, 7200);
            Ok(2)
        }
        _ => Err(format!(
            "Unknown `tuner --vault auth` option: '{flag}'. Usage: tuner --vault auth <site_key> [--url <url>] [--ttl <secs>] [--state-name <slug>] [--profile <name>]"
        )),
    }
}

/// Parses CLI arguments after `tuner --vault auth`.
pub fn parse_vault_auth_args(args: &[String]) -> Result<VaultAuthOptions, String> {
    let mut opts = VaultAuthOptions {
        site_key: String::new(),
        start_url: None,
        state_name: None,
        ttl_secs: DEFAULT_AUTH_TTL_SECS,
        no_tunnel: false,
    };
    let mut idx = 0;
    while idx < args.len() {
        let arg = args[idx].as_str();
        if arg.starts_with('-') {
            idx += consume_option_flag(args, idx, &mut opts)?;
        } else {
            if opts.site_key.is_empty() {
                opts.site_key = arg.trim().to_string();
            }
            idx += 1;
        }
    }
    if opts.site_key.is_empty() {
        return Err(
            "Missing <site_key>. Usage: tuner --vault auth <site_key> [--url <url>] [--ttl <secs>]"
                .to_string(),
        );
    }
    Ok(opts)
}

fn find_matching_vault_entry(store: &VaultStore, site_key: &str, slug: &str) -> Option<VaultEntry> {
    if let Ok(Some(exact)) = store.get_entry(site_key) {
        return Some(exact);
    }
    let entries = store.load_entries().ok()?;
    entries
        .into_iter()
        .find(|e| canonical_storage_slug(&e.site_key) == slug)
}

/// Resolves the target paths, start URL, and optional `VaultEntry` inside `vault_dir`.
pub fn resolve_auth_target(
    vault_dir: &Path,
    opts: &VaultAuthOptions,
) -> Result<ResolvedAuthTarget, String> {
    let slug = opts
        .state_name
        .clone()
        .unwrap_or_else(|| canonical_storage_slug(&opts.site_key));
    let store = VaultStore::new(vault_dir);
    let entry = find_matching_vault_entry(&store, &opts.site_key, &slug);

    let raw_url = if let Some(ref explicit) = opts.start_url {
        explicit.clone()
    } else if let Some(ref ent) = entry
        && !ent.url.trim().is_empty()
        && validate_http_url(&ent.url).is_ok()
    {
        ent.url.trim().to_string()
    } else {
        default_url_for_slug(&slug, &opts.site_key)
    };
    let start_url = validate_http_url(&raw_url)?;

    Ok(ResolvedAuthTarget {
        context_dir: vault_dir.join(format!("chromium_{slug}_ctx")),
        state_path: vault_dir.join(format!("{slug}_storage_state.json")),
        slug,
        start_url,
        entry,
    })
}
