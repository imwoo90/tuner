//! # Telegram Web Vault Session Manager and HTTP Handlers
//!
//! ## Overview
//! Manages 15-minute ephemeral web tokens for `/vault`, serves the encrypted credential
//! management UI (`vault_viewer.html`), and exposes REST endpoints (`GET/POST/DELETE`)
//! backed by [`crate::security::vault::VaultStore`].
//!
//! ## Search Tags
//! #telegram-vault, #web-vault-ui, #ephemeral-token, #credential-management

use crate::config::CliConfig;
use crate::security::vault::{current_unix_secs, VaultEntry, VaultEntryMasked, VaultStore};
use crate::security::vault_cli::{load_persisted_sessions, save_persisted_sessions, VaultWebSession};
use crate::workspace::paths::extract_profile_from_working_dir;
use axum::{
    extract::Path,
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    Json,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use tokio::sync::Mutex;

const VAULT_SESSION_TTL_SECS: u64 = 900; // 15 minutes

/// Manages active ephemeral Web Vault tokens for the current worker process.
pub struct VaultWebManager {
    sessions: Arc<Mutex<HashMap<String, VaultWebSession>>>,
    default_profile: Arc<Mutex<String>>,
    default_vault_dir: Arc<Mutex<PathBuf>>,
}

static VAULT_INSTANCE: OnceLock<Arc<VaultWebManager>> = OnceLock::new();

/// Returns the singleton [`VaultWebManager`] instance.
pub fn global_vault_manager() -> Arc<VaultWebManager> {
    VAULT_INSTANCE
        .get_or_init(|| Arc::new(VaultWebManager::new()))
        .clone()
}

impl Default for VaultWebManager {
    fn default() -> Self {
        Self::new()
    }
}

impl VaultWebManager {
    /// Creates a new manager initialized with fallback default profile paths.
    pub fn new() -> Self {
        let paths = crate::workspace::paths::resolve_paths(
            None,
            None,
            None,
            Some("default".to_string()),
        );
        let dir = paths.vault_dir();
        let initial = load_persisted_sessions(&dir);
        Self {
            sessions: Arc::new(Mutex::new(initial)),
            default_profile: Arc::new(Mutex::new("default".to_string())),
            default_vault_dir: Arc::new(Mutex::new(dir)),
        }
    }

    /// Initializes the worker's profile name and vault storage directory.
    pub async fn init_storage(&self, profile: String, vault_dir: PathBuf) {
        let disk = load_persisted_sessions(&vault_dir);
        {
            let mut g = self.sessions.lock().await;
            for (k, v) in disk {
                g.insert(k, v);
            }
        }
        {
            let mut p = self.default_profile.lock().await;
            *p = profile;
        }
        {
            let mut d = self.default_vault_dir.lock().await;
            *d = vault_dir;
        }
    }

    /// Returns the currently configured `(profile, vault_dir)`.
    pub async fn current_target(&self) -> (String, PathBuf) {
        let p = self.default_profile.lock().await.clone();
        let d = self.default_vault_dir.lock().await.clone();
        (p, d)
    }

    /// Creates a new 15-minute ephemeral session token for the given profile and vault directory.
    pub async fn create_session_for(&self, profile: String, vault_dir: PathBuf) -> String {
        let token = hex::encode(rand::random::<[u8; 16]>());
        let now = current_unix_secs();
        let mut guard = self.sessions.lock().await;
        guard.retain(|_, v| now.saturating_sub(v.created_at_secs) < VAULT_SESSION_TTL_SECS);
        guard.insert(
            token.clone(),
            VaultWebSession {
                created_at_secs: now,
                profile,
                vault_dir: vault_dir.clone(),
            },
        );
        save_persisted_sessions(&vault_dir, &guard);
        token
    }

    /// Resolves an active token into `(profile_name, VaultStore, remaining_ttl_secs)`.
    pub async fn resolve_session(&self, token: &str) -> Option<(String, VaultStore, u64)> {
        let now = current_unix_secs();
        let def_dir = self.default_vault_dir.lock().await.clone();
        let mut guard = self.sessions.lock().await;
        if !guard.contains_key(token) {
            for (k, v) in load_persisted_sessions(&def_dir) {
                guard.insert(k, v);
            }
        }
        guard.retain(|_, v| now.saturating_sub(v.created_at_secs) < VAULT_SESSION_TTL_SECS);
        let sess = guard.get(token)?;
        let elapsed = now.saturating_sub(sess.created_at_secs);
        let rem = VAULT_SESSION_TTL_SECS.saturating_sub(elapsed);
        Some((
            sess.profile.clone(),
            VaultStore::new(sess.vault_dir.clone()),
            rem,
        ))
    }

    /// Immediately invalidates a token session.
    pub async fn lock_session(&self, token: &str) -> bool {
        let def_dir = self.default_vault_dir.lock().await.clone();
        let mut guard = self.sessions.lock().await;
        for (k, v) in load_persisted_sessions(&def_dir) {
            guard.entry(k).or_insert(v);
        }
        if let Some(removed) = guard.remove(token) {
            save_persisted_sessions(&removed.vault_dir, &guard);
            true
        } else {
            false
        }
    }
}

fn entries_response(store: &VaultStore) -> Response {
    match store.load_entries() {
        Ok(entries) => Json(serde_json::json!({ "entries": entries })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// Renders the HTML Web Vault interface for `/vault/:token`.
pub async fn handle_vault_page(Path(token): Path<String>) -> Response {
    let Some((profile, store, ttl_secs)) = global_vault_manager().resolve_session(&token).await else {
        return (StatusCode::NOT_FOUND, "Vault session expired or locked. Run /vault in Telegram.").into_response();
    };
    let entries = match store.load_entries() {
        Ok(v) => v,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let bootstrap = serde_json::json!({
        "token": token,
        "profile": profile,
        "ttl_secs": ttl_secs,
        "entries": entries,
    });
    let json_str = serde_json::to_string(&bootstrap).unwrap_or_default().replace("</", "<\\/");
    let html = include_str!("vault_viewer.html").replace("__VAULT_BOOTSTRAP__", &json_str);
    let mut res = Html(html).into_response();
    if let Ok(hv) = header::HeaderValue::from_str("no-store, no-cache, must-revalidate") {
        res.headers_mut().insert(header::CACHE_CONTROL, hv);
    }
    res
}

/// Returns JSON credential entries for `/vault/:token/api/entries`.
pub async fn handle_vault_list(Path(token): Path<String>) -> Response {
    let Some((profile, store, ttl_secs)) = global_vault_manager().resolve_session(&token).await else {
        return (StatusCode::UNAUTHORIZED, "Session expired or locked").into_response();
    };
    match store.load_entries() {
        Ok(entries) => Json(serde_json::json!({ "profile": profile, "ttl_secs": ttl_secs, "entries": entries })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// Creates or updates a credential entry via `POST /vault/:token/api/entries`.
pub async fn handle_vault_upsert(Path(token): Path<String>, Json(payload): Json<VaultEntry>) -> Response {
    let Some((_, store, _)) = global_vault_manager().resolve_session(&token).await else {
        return (StatusCode::UNAUTHORIZED, "Session expired or locked").into_response();
    };
    if let Err(e) = store.upsert_entry(payload) {
        return (StatusCode::BAD_REQUEST, e).into_response();
    }
    entries_response(&store)
}

/// Deletes a credential entry via `DELETE /vault/:token/api/entries/:site_key`.
pub async fn handle_vault_delete(Path((token, site_key)): Path<(String, String)>) -> Response {
    let Some((_, store, _)) = global_vault_manager().resolve_session(&token).await else {
        return (StatusCode::UNAUTHORIZED, "Session expired or locked").into_response();
    };
    if let Err(e) = store.delete_entry(&site_key) {
        return (StatusCode::BAD_REQUEST, e).into_response();
    }
    entries_response(&store)
}

/// Invalidates an active vault session token via `POST /vault/:token/api/lock`.
pub async fn handle_vault_lock(Path(token): Path<String>) -> Response {
    let locked = global_vault_manager().lock_session(&token).await;
    Json(serde_json::json!({ "locked": locked })).into_response()
}

fn format_vault_message(profile: &str, masked: &[VaultEntryMasked]) -> String {
    let site_summary = if masked.is_empty() {
        "• <i>등록된 사이트 계정이 없습니다. 아래 버튼을 눌러 추가하세요.</i>".to_string()
    } else {
        masked
            .iter()
            .map(|m| {
                format!(
                    "• <code>{}</code> — {} (ID: <code>{}</code>)",
                    html_escape::encode_text(&m.site_key),
                    html_escape::encode_text(&m.title),
                    html_escape::encode_text(&m.masked_username),
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "🔐 <b>[Tuner 자격증명 금고]</b> (프로필: <code>{}</code>)\n\n\
        <b>등록된 사이트 ({}개):</b>\n{}\n\n\
        🛡️ <b>보안 안내:</b>\n\
        • 비밀번호는 프로필 전용 <code>XSalsa20-Poly1305</code> 키로 암호화 저장됩니다.\n\
        • AI 에이전트는 비밀번호 원문을 읽지 않고 <code>tuner --vault exec &lt;사이트키&gt;</code>로 안전하게 로그인합니다.\n\
        • 아래 관리 링크는 <b>15분간 유효</b>하며, 사용 후 웹페이지 우측 상단 <b>[🔒 즉시 잠금]</b>으로 바로 닫을 수 있습니다.",
        html_escape::encode_text(profile),
        masked.len(),
        site_summary
    )
}

/// Handles the `/vault` slash command in Telegram.
pub async fn handle_vault_command(
    bot: &Bot,
    msg: &Message,
    config: &CliConfig,
) -> Result<(), teloxide::RequestError> {
    let mgr = global_vault_manager();
    let (mut profile, mut vault_dir) = mgr.current_target().await;
    if let Some((det_prof, det_dir)) = extract_profile_from_working_dir(&config.working_dir) {
        profile = det_prof;
        vault_dir = det_dir;
    }

    let store = VaultStore::new(&vault_dir);
    let masked = store.list_masked().unwrap_or_default();
    let token = mgr.create_session_for(profile.clone(), vault_dir).await;
    let vault_url = super::review::global_review_manager()
        .get_vault_url(&token)
        .await;
    let text = format_vault_message(&profile, &masked);

    let mut buttons = Vec::new();
    if let Ok(parsed_url) = url::Url::parse(&vault_url) {
        buttons.push(vec![InlineKeyboardButton::url(
            "🔐 자격증명 금고 열기 (Web Vault)".to_string(),
            parsed_url,
        )]);
    }
    let markup = InlineKeyboardMarkup::new(buttons);

    let mut req = bot
        .send_message(msg.chat.id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(markup);
    if let Some(tid) = msg.thread_id {
        req = req.message_thread_id(tid);
    }
    req.await?;
    Ok(())
}
