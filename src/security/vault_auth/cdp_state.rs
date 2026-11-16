//! # Playwright-Compatible `storage_state.json` Persistence (`cdp_state.rs`)
//!
//! ## Overview
//! Imports existing cookies into a live Chrome CDP session and atomically exports
//! browser cookies plus page `localStorage` origins in Playwright's exact
//! `{"cookies": [...], "origins": [...]}` schema (`chmod 0600`).
//!
//! ## Search Tags
//! #storage-state, #playwright-cookies, #cdp-cookies, #atomic-write

use super::cdp::{cdp_call_ws, list_page_targets};
use super::proxy::set_mode_0600;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;

/// Summary of an exported Playwright `storage_state.json` snapshot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportedStateSummary {
    /// Total number of cookies persisted.
    pub cookie_count: usize,
    /// Total number of `localStorage` origins captured.
    pub origin_count: usize,
    /// Last active page URL at snapshot time.
    pub final_url: String,
}

/// Validates that a user/CLI navigation URL uses strictly `http://` or `https://` (`file://` forbidden).
///
/// # Examples
/// ```
/// use tuner::security::vault_auth::cdp_state::validate_http_url;
/// assert!(validate_http_url("https://www.aliexpress.com/").is_ok());
/// assert!(validate_http_url("file:///etc/passwd").is_err());
/// ```
pub fn validate_http_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let parsed = url::Url::parse(trimmed).map_err(|e| format!("Invalid URL '{trimmed}': {e}"))?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed.to_string()),
        other => Err(format!(
            "Forbidden URL scheme '{other}://': only http:// and https:// are permitted."
        )),
    }
}

/// Loads saved cookies from an existing Playwright `storage_state.json` into Chrome via CDP.
pub async fn seed_existing_storage_state(browser_ws_url: &str, state_path: &Path) {
    let Ok(raw) = std::fs::read_to_string(state_path) else {
        return;
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&raw) else {
        return;
    };
    let Some(cookies) = parsed.get("cookies").and_then(|c| c.as_array()) else {
        return;
    };
    if cookies.is_empty() {
        return;
    }
    let mut cdp_cookies = Vec::with_capacity(cookies.len());
    for c in cookies {
        let Some(name) = c.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(val) = c.get("value").and_then(Value::as_str) else {
            continue;
        };
        let Some(domain) = c.get("domain").and_then(Value::as_str) else {
            continue;
        };
        let mut obj = json!({
            "name": name,
            "value": val,
            "domain": domain,
            "path": c.get("path").and_then(Value::as_str).unwrap_or("/"),
            "secure": c.get("secure").and_then(Value::as_bool).unwrap_or(false),
            "httpOnly": c.get("httpOnly").and_then(Value::as_bool).unwrap_or(false),
        });
        if let Some(exp) = c.get("expires").and_then(Value::as_f64)
            && exp > 0.0
            && let Some(map) = obj.as_object_mut()
        {
            map.insert("expires".to_string(), json!(exp));
        }
        if let Some(ss) = c.get("sameSite").and_then(Value::as_str)
            && matches!(ss, "Strict" | "Lax" | "None")
            && let Some(map) = obj.as_object_mut()
        {
            map.insert("sameSite".to_string(), json!(ss));
        }
        cdp_cookies.push(obj);
    }
    if !cdp_cookies.is_empty() {
        let _ = cdp_call_ws(
            browser_ws_url,
            "Storage.setCookies",
            json!({ "cookies": cdp_cookies }),
        )
        .await;
    }
}

fn normalize_playwright_cookie(raw: &Value) -> Option<Value> {
    let name = raw.get("name")?.as_str()?;
    let value = raw.get("value")?.as_str()?;
    let domain = raw.get("domain")?.as_str()?;
    let path = raw.get("path").and_then(Value::as_str).unwrap_or("/");
    let expires = raw.get("expires").and_then(Value::as_f64).unwrap_or(-1.0);
    let http_only = raw.get("httpOnly").and_then(Value::as_bool).unwrap_or(false);
    let secure = raw.get("secure").and_then(Value::as_bool).unwrap_or(false);
    let same_site = match raw.get("sameSite").and_then(Value::as_str) {
        Some("Strict") => "Strict",
        Some("None") => "None",
        _ => "Lax",
    };
    Some(json!({
        "name": name,
        "value": value,
        "domain": domain,
        "path": path,
        "expires": if expires <= 0.0 { -1.0 } else { expires },
        "httpOnly": http_only,
        "secure": secure,
        "sameSite": same_site,
    }))
}

async fn collect_open_page_origins(cdp_port: u16) -> (Vec<Value>, String) {
    let targets = list_page_targets(cdp_port).await;
    let final_url = targets
        .first()
        .map(|t| t.url.clone())
        .unwrap_or_default();
    let mut seen = HashSet::new();
    let mut origins = Vec::new();
    let expr = r#"(() => {
        try {
            const origin = window.location.origin;
            if (!origin || origin === 'null' || !origin.startsWith('http')) return null;
            const ls = [];
            for (let i = 0; i < window.localStorage.length; i++) {
                const k = window.localStorage.key(i);
                if (k !== null) ls.push({ name: k, value: String(window.localStorage.getItem(k) ?? '') });
            }
            return ls.length ? { origin, localStorage: ls } : null;
        } catch (_) { return null; }
    })()"#;

    for t in targets {
        if !t.url.starts_with("http://") && !t.url.starts_with("https://") {
            continue;
        }
        let Ok(res) = cdp_call_ws(
            &t.ws_url,
            "Runtime.evaluate",
            json!({ "expression": expr, "returnByValue": true }),
        )
        .await
        else {
            continue;
        };
        let val = &res["result"]["value"];
        if let Some(origin) = val.get("origin").and_then(Value::as_str)
            && seen.insert(origin.to_string())
        {
            origins.push(val.clone());
        }
    }
    (origins, final_url)
}

/// Exports cookies and `localStorage` origins to `state_path` (`0600`) in Playwright format.
pub async fn export_storage_state(
    cdp_port: u16,
    browser_ws_url: &str,
    state_path: &Path,
) -> Result<ExportedStateSummary, String> {
    let raw_cookies = cdp_call_ws(browser_ws_url, "Storage.getCookies", json!({})).await?;
    let cookies: Vec<Value> = raw_cookies
        .get("cookies")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(normalize_playwright_cookie).collect())
        .unwrap_or_default();

    let (origins, final_url) = collect_open_page_origins(cdp_port).await;
    let doc = json!({
        "cookies": cookies,
        "origins": origins,
    });
    if let Some(parent) = state_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let serialized = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    let tmp = state_path.with_extension("json.tmp");
    std::fs::write(&tmp, serialized)
        .map_err(|e| format!("Failed to write storage_state tmp {:?}: {}", tmp, e))?;
    set_mode_0600(&tmp);
    std::fs::rename(&tmp, state_path)
        .map_err(|e| format!("Failed to commit storage_state {:?}: {}", state_path, e))?;
    set_mode_0600(state_path);

    Ok(ExportedStateSummary {
        cookie_count: cookies.len(),
        origin_count: origins.len(),
        final_url,
    })
}

/// Requests graceful browser shutdown over CDP so SQLite cookies flush cleanly to disk.
pub async fn send_graceful_browser_close(browser_ws_url: &str) {
    let _ = cdp_call_ws(browser_ws_url, "Browser.close", json!({})).await;
}
