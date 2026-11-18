//! # In-Dashboard Telegram Bot Setup & Hot-Connect API (`api_telegram.rs`)
//!
//! ## Overview
//! Validates `@BotFather` tokens via `getMe`, persists `telegram_token`, `allowed_user_ids`,
//! and `allowed_group_ids` to `<profile>/config/config.json` (`0600`), and hot-spawns or
//! disconnects the live Telegram polling dispatcher with zero Web Dashboard downtime.
//!
//! ## Search Tags
//! #telegram-connect, #hot-dispatcher, #web-standalone-bridge

use super::auth_http::{json_err, require_auth, require_mutation_auth};
use super::events::WebEvent;
use super::{resolve_ctx, WebAppCtx};
use crate::messenger::telegram::runner::spawn_telegram_dispatcher;
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use teloxide::Bot;
use tokio::sync::Mutex;
use tokio::task::AbortHandle;

struct HotTelegramState {
    abort: Option<AbortHandle>,
    username: Option<String>,
}

static HOT_TG: OnceLock<Mutex<HotTelegramState>> = OnceLock::new();

fn hot_tg() -> &'static Mutex<HotTelegramState> {
    HOT_TG.get_or_init(|| Mutex::new(HotTelegramState { abort: None, username: None }))
}

fn profile_config_path(ctx: &WebAppCtx) -> PathBuf {
    ctx.tuner_home.join("profiles").join(&ctx.profile).join("config/config.json")
}

/// Reads current `(allowed_user_ids, allowed_group_ids)` from `<profile>/config/config.json` or `ctx.config`.
pub fn read_live_allowed_ids(ctx: &WebAppCtx) -> (Vec<i64>, Vec<i64>) {
    let cfg_path = profile_config_path(ctx);
    if let Ok(raw) = std::fs::read_to_string(&cfg_path)
        && let Ok(val) = serde_json::from_str::<Value>(&raw)
    {
        let arr_i64 = |k: &str, fb: &[i64]| {
            val.get(k)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_i64).collect::<Vec<_>>())
                .unwrap_or_else(|| fb.to_vec())
        };
        return (
            arr_i64("allowed_user_ids", &ctx.config.allowed_user_ids),
            arr_i64("allowed_group_ids", &ctx.config.allowed_group_ids),
        );
    }
    (ctx.config.allowed_user_ids.clone(), ctx.config.allowed_group_ids.clone())
}

/// Resolves the connected Telegram bot's `@username` (without `@`), caching after first lookup.
pub async fn resolve_bot_username(ctx: &WebAppCtx) -> Option<String> {
    let bot = ctx.bot_clone().await?;
    if let Some(cached) = hot_tg().lock().await.username.clone() {
        return Some(cached);
    }
    if let Ok(Some(uname)) = verify_bot_token_get_me(&bot, 3).await {
        hot_tg().lock().await.username = Some(uname.clone());
        return Some(uname);
    }
    None
}

type AllowedIdPair = (Vec<i64>, Vec<i64>);

fn parse_allowed_ids_input(raw: Option<&str>) -> Result<Option<AllowedIdPair>, String> {
    let Some(txt) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let mut users = Vec::new();
    let mut groups = Vec::new();
    for part in txt.split([',', ' ', '\n', '\t']).map(str::trim).filter(|s| !s.is_empty()) {
        let id = part
            .parse::<i64>()
            .map_err(|_| format!("Invalid numeric Telegram ID `{part}`"))?;
        if id > 0 && !users.contains(&id) {
            users.push(id);
        } else if id < 0 && !groups.contains(&id) {
            groups.push(id);
        }
    }
    Ok(Some((users, groups)))
}

fn persist_telegram_config(
    ctx: &WebAppCtx,
    token: &str,
    ids: Option<&(Vec<i64>, Vec<i64>)>,
) -> Result<(Vec<i64>, Vec<i64>), String> {
    let path = profile_config_path(ctx);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut val: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    let Some(obj) = val.as_object_mut() else {
        return Err("Invalid config.json root object".to_string());
    };
    obj.insert("telegram_token".to_string(), json!(token));
    if let Some((u, g)) = ids {
        obj.insert("allowed_user_ids".to_string(), json!(u));
        obj.insert("allowed_group_ids".to_string(), json!(g));
    }
    let serialized = serde_json::to_string_pretty(&val).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, format!("{serialized}\n")).map_err(|e| e.to_string())?;
    let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(read_live_allowed_ids(ctx))
}

/// Request payload for `POST /dashboard/api/telegram/connect`.
#[derive(Deserialize)]
pub struct ConnectTelegramPayload {
    /// `@BotFather` HTTP API token (`123456:ABC...`).
    pub token: String,
    /// Optional comma-separated numeric Telegram User (`>0`) or Group (`<0`) IDs.
    #[serde(default)]
    pub allowed_ids: Option<String>,
    /// Optional loopback mock API base URL (`http://127.0.0.1:...`) for isolated tests.
    #[serde(default)]
    pub api_url: Option<String>,
}

async fn handle_telegram_status(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_auth(&ctx, &headers).await {
        return *r;
    }
    let connected = ctx.bot_clone().await.is_some();
    let bot_username = resolve_bot_username(&ctx).await;
    let (users, groups) = read_live_allowed_ids(&ctx);
    Json(json!({
        "connected": connected,
        "bot_username": bot_username,
        "allowed_user_ids": users,
        "allowed_group_ids": groups,
    }))
    .into_response()
}

async fn verify_bot_token_get_me(bot: &Bot, secs: u64) -> Result<Option<String>, String> {
    let base = bot.api_url().as_str().trim_end_matches('/').to_string();
    let url = format!("{base}/bot{}/getMe", bot.token());
    let client = reqwest::Client::builder().timeout(Duration::from_secs(secs)).build().map_err(|e| e.to_string())?;
    let resp = client.get(&url).send().await.map_err(|e| format!("Failed to reach Telegram API: {e}"))?;
    let val: Value = resp.json().await.map_err(|e| format!("Invalid Telegram API response: {e}"))?;
    if val.get("ok").and_then(Value::as_bool) != Some(true) {
        let desc = val.get("description").and_then(Value::as_str).unwrap_or("Unauthorized Bot Token");
        return Err(format!("Telegram rejected Bot Token ({desc})"));
    }
    Ok(val.get("result").and_then(|r| r.get("username")).and_then(Value::as_str).map(str::to_string))
}

async fn activate_hot_telegram_bot(
    ctx: &Arc<WebAppCtx>,
    tok: &str,
    bot: Bot,
    uname: Option<String>,
    users: Vec<i64>,
    groups: Vec<i64>,
) {
    let mut next_cfg = (*ctx.config).clone();
    next_cfg.telegram_token = tok.to_string();
    next_cfg.transport = "tg".to_string();
    next_cfg.allowed_user_ids = users;
    next_cfg.allowed_group_ids = groups;
    let bot_info = Arc::new(crate::messenger::telegram::topic_cache::BotInfo {
        username: uname.clone(),
    });
    ctx.set_bot(bot.clone()).await;
    {
        let mut g = hot_tg().lock().await;
        if let Some(prev) = g.abort.take() {
            prev.abort();
        }
        let jh = spawn_telegram_dispatcher(ctx.clone(), Arc::new(next_cfg), bot, bot_info);
        g.abort = Some(jh.abort_handle());
        g.username = uname.clone();
    }
    ctx.events
        .publish(WebEvent::TelegramStatusUpdated {
            connected: true,
            bot_username: uname,
        })
        .await;
}

fn build_candidate_bot(tok: &str, api_url: Option<&str>) -> Result<Bot, String> {
    let mut bot = Bot::new(tok);
    if let Some(raw_u) = api_url.map(str::trim).filter(|s| !s.is_empty()) {
        let parsed = url::Url::parse(raw_u).map_err(|e| format!("Invalid api_url: {e}"))?;
        if !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost")) {
            return Err("Custom api_url is restricted to loopback testing".to_string());
        }
        bot = bot.set_api_url(parsed);
    }
    Ok(bot)
}

async fn handle_telegram_connect(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
    Json(payload): Json<ConnectTelegramPayload>,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    let tok = payload.token.trim();
    if tok.is_empty() || !tok.contains(':') {
        return json_err(StatusCode::BAD_REQUEST, "Valid Telegram Bot Token (e.g. 123456:ABC...) is required");
    }
    let parsed_ids = match parse_allowed_ids_input(payload.allowed_ids.as_deref()) {
        Ok(v) => v,
        Err(e) => return json_err(StatusCode::BAD_REQUEST, &e),
    };
    let bot = match build_candidate_bot(tok, payload.api_url.as_deref()) {
        Ok(b) => b,
        Err(e) => return json_err(StatusCode::BAD_REQUEST, &e),
    };
    let uname = match verify_bot_token_get_me(&bot, 10).await {
        Ok(u) => u,
        Err(msg) => return json_err(StatusCode::BAD_REQUEST, &msg),
    };
    let (users, groups) = match persist_telegram_config(&ctx, tok, parsed_ids.as_ref()) {
        Ok(pair) => pair,
        Err(e) => return json_err(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    activate_hot_telegram_bot(&ctx, tok, bot, uname.clone(), users, groups).await;
    Json(json!({ "ok": true, "connected": true, "bot_username": uname })).into_response()
}

async fn handle_telegram_disconnect(
    ext: Option<Extension<Arc<WebAppCtx>>>,
    headers: HeaderMap,
) -> Response {
    let ctx = match resolve_ctx(ext) {
        Ok(c) => c,
        Err(r) => return *r,
    };
    if let Err(r) = require_mutation_auth(&ctx, &headers).await {
        return *r;
    }
    {
        let mut g = hot_tg().lock().await;
        if let Some(prev) = g.abort.take() {
            prev.abort();
        }
        g.username = None;
    }
    *ctx.bot.write().await = None;
    let _ = persist_telegram_config(&ctx, "", None);
    ctx.events
        .publish(WebEvent::TelegramStatusUpdated {
            connected: false,
            bot_username: None,
        })
        .await;
    Json(json!({ "ok": true, "connected": false })).into_response()
}

/// Builds the `/dashboard/api/telegram/*` sub-router.
pub fn telegram_routes() -> Router {
    Router::new()
        .route("/dashboard/api/telegram/status", get(handle_telegram_status))
        .route("/dashboard/api/telegram/connect", post(handle_telegram_connect))
        .route("/dashboard/api/telegram/disconnect", post(handle_telegram_disconnect))
}
