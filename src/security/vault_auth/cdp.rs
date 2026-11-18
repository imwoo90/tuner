//! # Pure Rust Chrome DevTools Protocol Controller (`cdp.rs`)
//!
//! ## Overview
//! Drives the active MRU Chrome page target (`type == "page"`) over a persistent loopback CDP
//! WebSocket session—preserving `Emulation.setDeviceMetricsOverride` (`760 × 980` desktop layout)
//! and `Page.addScriptToEvaluateOnNewDocument` across navigations and popups while eliminating
//! ephemeral socket churn, blocking `file://` URLs, and routing input via [`super::cdp_input`].
//!
//! ## Search Tags
//! #cdp-controller, #persistent-cdp-session, #mru-tab-tracking, #screencast-jpeg

use super::browser::{VIEWPORT_HEIGHT, VIEWPORT_WIDTH};
use super::cdp_input::{
    cycle_browser_tab, dispatch_nav_or_key, dispatch_pointer_or_wheel, dispatch_text_or_credential,
    extract_copyable_text, probe_oauth_callback_code,
};
use super::cdp_state::validate_http_url;
use base64::Engine;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

type CdpWsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Metadata for an open Chrome `"page"` target returned by `/json/list` (MRU-ordered).
#[derive(Clone, Debug, Deserialize)]
pub struct CdpPageTarget {
    /// Target identifier.
    pub id: String,
    /// Target type (`"page"`, `"browser_ui"`, etc.).
    #[serde(rename = "type")]
    pub target_type: String,
    /// Current target URL.
    pub url: String,
    /// Page WebSocket debugger endpoint.
    #[serde(rename = "webSocketDebuggerUrl", default)]
    pub ws_url: String,
}

/// Live frame snapshot returned to the WebSocket streamer loop.
pub struct FrameSnapshot {
    /// Decoded JPEG image bytes (`760 × 980`).
    pub jpeg_bytes: Vec<u8>,
    /// Current URL of the active MRU page.
    pub url: String,
    /// Number of open user `"page"` tabs.
    pub tab_count: usize,
}

struct ActivePageConn {
    target_id: String,
    ws: CdpWsStream,
    next_id: u64,
}

/// Stateful CDP controller maintaining a persistent session to the active MRU page target.
pub struct CdpController {
    cdp_port: u16,
    active_conn: Mutex<Option<ActivePageConn>>,
    /// Decrypted username held strictly in Rust RAM (never echoed or logged).
    vault_username: Option<String>,
    /// Decrypted password held strictly in Rust RAM (never echoed or logged).
    vault_password: Option<String>,
}

async fn send_rpc_on_stream(
    ws: &mut CdpWsStream,
    req_id: u64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let fut = async {
        let req = json!({ "id": req_id, "method": method, "params": params });
        ws.send(Message::Text(req.to_string()))
            .await
            .map_err(|e| format!("__BROKEN__:CDP send error: {e}"))?;
        while let Some(msg) = ws.next().await {
            let msg = msg.map_err(|e| format!("__BROKEN__:CDP recv error: {e}"))?;
            match msg {
                Message::Ping(p) => {
                    let _ = ws.send(Message::Pong(p)).await;
                }
                Message::Close(_) => return Err(format!("__BROKEN__:CDP closed ({method})")),
                Message::Text(txt) => {
                    if let Ok(val) = serde_json::from_str::<Value>(&txt)
                        && val.get("id").and_then(Value::as_u64) == Some(req_id)
                    {
                        if let Some(err) = val.get("error") {
                            return Err(format!("CDP {method} error: {err}"));
                        }
                        return Ok(val.get("result").cloned().unwrap_or(Value::Null));
                    }
                }
                _ => {}
            }
        }
        Err(format!("__BROKEN__:CDP EOF ({method})"))
    };
    tokio::time::timeout(Duration::from_secs(6), fut)
        .await
        .map_err(|_| format!("__BROKEN__:CDP {method} timed out"))?
}

/// Sends a single one-off JSON-RPC command to a CDP WebSocket URL (used for browser/export queries).
pub async fn cdp_call_ws(ws_url: &str, method: &str, params: Value) -> Result<Value, String> {
    let (mut ws, _) = tokio::time::timeout(Duration::from_secs(5), connect_async(ws_url))
        .await
        .map_err(|_| format!("CDP connect timeout ({method})"))?
        .map_err(|e| format!("CDP connect error: {e}"))?;
    let res = send_rpc_on_stream(&mut ws, 1, method, params)
        .await
        .map_err(|e| e.trim_start_matches("__BROKEN__:").to_string());
    let _ = ws.close(None).await;
    res
}

async fn open_configured_page_conn(target: &CdpPageTarget) -> Result<ActivePageConn, String> {
    let (mut ws, _) = tokio::time::timeout(Duration::from_secs(5), connect_async(&target.ws_url))
        .await
        .map_err(|_| "CDP page connect timeout".to_string())?
        .map_err(|e| format!("CDP page connect error: {e}"))?;
    let stealth_js = "Object.defineProperty(navigator, 'webdriver', {get: () => undefined});";
    let _ = send_rpc_on_stream(&mut ws, 1, "Page.enable", json!({})).await;
    let _ = send_rpc_on_stream(
        &mut ws,
        2,
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": stealth_js }),
    )
    .await;
    let _ = send_rpc_on_stream(
        &mut ws,
        3,
        "Runtime.evaluate",
        json!({ "expression": stealth_js }),
    )
    .await;
    let _ = send_rpc_on_stream(
        &mut ws,
        4,
        "Emulation.setDeviceMetricsOverride",
        json!({
            "width": VIEWPORT_WIDTH,
            "height": VIEWPORT_HEIGHT,
            "deviceScaleFactor": 1,
            "mobile": false,
        }),
    )
    .await;
    Ok(ActivePageConn {
        target_id: target.id.clone(),
        ws,
        next_id: 10,
    })
}

/// Queries `http://127.0.0.1:<cdp_port>/json/list` and returns user `"page"` targets in MRU order.
pub async fn list_page_targets(cdp_port: u16) -> Vec<CdpPageTarget> {
    let url = format!("http://127.0.0.1:{cdp_port}/json/list");
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    else {
        return Vec::new();
    };
    let Ok(resp) = client.get(&url).send().await else {
        return Vec::new();
    };
    let Ok(all) = resp.json::<Vec<CdpPageTarget>>().await else {
        return Vec::new();
    };
    let pages: Vec<CdpPageTarget> = all
        .into_iter()
        .filter(|t| t.target_type == "page" && !t.ws_url.is_empty())
        .collect();
    let non_internal: Vec<CdpPageTarget> = pages
        .iter()
        .filter(|t| !t.url.starts_with("chrome://") && !t.url.starts_with("devtools://"))
        .cloned()
        .collect();
    if non_internal.is_empty() {
        pages
    } else {
        non_internal
    }
}

impl CdpController {
    /// Creates a new controller bound to `cdp_port` and optional in-memory `VaultStore` credentials.
    pub fn new(
        cdp_port: u16,
        vault_username: Option<String>,
        vault_password: Option<String>,
    ) -> Self {
        Self {
            cdp_port,
            active_conn: Mutex::new(None),
            vault_username: vault_username.filter(|s| !s.is_empty()),
            vault_password: vault_password.filter(|s| !s.is_empty()),
        }
    }

    /// Returns whether a non-empty username is available in RAM.
    pub fn has_username(&self) -> bool {
        self.vault_username.is_some()
    }

    /// Returns whether a non-empty password is available in RAM.
    pub fn has_password(&self) -> bool {
        self.vault_password.is_some()
    }

    /// Executes a CDP command over the persistent WebSocket bound to `target`.
    pub async fn call_page(
        &self,
        target: &CdpPageTarget,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        let mut guard = self.active_conn.lock().await;
        if guard.as_ref().map(|c| c.target_id.as_str()) != Some(target.id.as_str()) {
            *guard = Some(open_configured_page_conn(target).await?);
        }
        let conn = guard.as_mut().ok_or_else(|| "Missing CDP conn".to_string())?;
        let req_id = conn.next_id;
        conn.next_id = conn.next_id.saturating_add(1);
        match send_rpc_on_stream(&mut conn.ws, req_id, method, params).await {
            Ok(val) => Ok(val),
            Err(err) => {
                if err.starts_with("__BROKEN__:") {
                    *guard = None;
                }
                Err(err.trim_start_matches("__BROKEN__:").to_string())
            }
        }
    }

    async fn resolve_active_page(&self) -> Result<(CdpPageTarget, usize), String> {
        let pages = list_page_targets(self.cdp_port).await;
        let count = pages.len();
        let target = pages
            .into_iter()
            .next()
            .ok_or_else(|| "No active browser page tab available".to_string())?;
        if target.url.starts_with("file://") {
            let _ = self
                .call_page(&target, "Page.navigate", json!({ "url": "about:blank" }))
                .await;
            return Err("Blocked forbidden file:// navigation".to_string());
        }
        Ok((target, count))
    }

    /// Configures the initial tab (`760 × 980` + stealth) and navigates to `start_url`.
    pub async fn navigate_initial(&self, start_url: &str) -> Result<(), String> {
        let safe_url = validate_http_url(start_url)?;
        let (target, _) = self.resolve_active_page().await?;
        let _ = self
            .call_page(&target, "Page.navigate", json!({ "url": safe_url }))
            .await?;
        Ok(())
    }

    /// Captures a `quality: 68` JPEG frame from the active MRU page tab.
    pub async fn capture_frame(&self) -> Result<FrameSnapshot, String> {
        let (target, tab_count) = self.resolve_active_page().await?;
        let res = self
            .call_page(
                &target,
                "Page.captureScreenshot",
                json!({ "format": "jpeg", "quality": 68 }),
            )
            .await?;
        let b64 = res
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| "Missing screenshot base64 payload".to_string())?;
        let jpeg_bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| format!("Screenshot base64 decode error: {e}"))?;
        Ok(FrameSnapshot {
            jpeg_bytes,
            url: target.url,
            tab_count,
        })
    }

    /// Executes a single user UI action JSON message from the remote viewer WebSocket.
    pub async fn dispatch_action(&self, action: &Value) -> Result<Option<String>, String> {
        let act = action.get("type").and_then(Value::as_str).unwrap_or("");
        if act == "cycle_tab" {
            return cycle_browser_tab(self.cdp_port).await;
        }
        let (target, _) = self.resolve_active_page().await?;
        match act {
            "mousedown" | "mousemove" | "mouseup" | "wheel" => {
                dispatch_pointer_or_wheel(self, &target, act, action).await?;
                Ok(None)
            }
            "text" | "vault_user" | "vault_pass" => {
                dispatch_text_or_credential(
                    self,
                    &target,
                    act,
                    action,
                    self.vault_username.as_deref(),
                    self.vault_password.as_deref(),
                )
                .await
            }
            _ => dispatch_nav_or_key(self, &target, act, action).await,
        }
    }

    /// Extracts selected text, visible code box content, or remote clipboard from the active tab.
    pub async fn read_copyable_text(&self) -> Result<Option<String>, String> {
        let (target, _) = self.resolve_active_page().await?;
        extract_copyable_text(self, &target).await
    }

    /// Probes the active tab for an OAuth callback authorization code (`[data-code-box]`, `?code=`, etc.).
    pub async fn probe_oauth_code(&self) -> Option<String> {
        let (target, _) = self.resolve_active_page().await.ok()?;
        probe_oauth_callback_code(self, &target).await
    }
}
