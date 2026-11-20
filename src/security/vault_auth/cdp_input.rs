//! # CDP Pointer, Wheel, Keyboard & Vault Input Helpers (`cdp_input.rs`)
//!
//! ## Overview
//! Translates WebSocket UI actions (`mousedown`, `mousemove`, `mouseup`, `wheel`, `text`,
//! `vault_user`, `vault_pass`, `key`, `goto`, `reload`, `back`) into Chrome DevTools Protocol
//! JSON-RPC calls over the persistent [`super::cdp::CdpController`] session while keeping
//! decrypted Vault credentials strictly inside Rust RAM.
//!
//! ## Search Tags
//! #cdp-input, #mouse-dispatch, #keyboard-dispatch, #vault-typing

use super::browser::{VIEWPORT_HEIGHT, VIEWPORT_WIDTH};
use super::cdp::{list_page_targets, CdpController, CdpPageTarget};
use super::cdp_state::validate_http_url;
use serde_json::{json, Value};
use std::time::Duration;

/// Activates the least-recently-used tab so mobile users can cycle between main and popup windows.
pub async fn cycle_browser_tab(cdp_port: u16) -> Result<Option<String>, String> {
    let pages = list_page_targets(cdp_port).await;
    if let Some(last) = pages.last()
        && pages.len() > 1
    {
        let url = format!("http://127.0.0.1:{cdp_port}/json/activate/{}", last.id);
        if let Ok(c) = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
        {
            let _ = c.get(&url).send().await;
        }
        return Ok(Some(format!("🗂️ 탭 전환됨 ({})", last.url)));
    }
    Ok(None)
}

/// Dispatches a pointer (`mousedown`, `mousemove`, `mouseup`) or `wheel` event over CDP.
pub async fn dispatch_pointer_or_wheel(
    cdp: &CdpController,
    target: &CdpPageTarget,
    act: &str,
    action: &Value,
) -> Result<(), String> {
    if act == "wheel" {
        let dy = action.get("dy").and_then(Value::as_f64).unwrap_or(200.0);
        cdp.call_page(
            target,
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseWheel",
                "x": VIEWPORT_WIDTH as f64 / 2.0,
                "y": VIEWPORT_HEIGHT as f64 / 2.0,
                "deltaX": 0.0,
                "deltaY": dy,
            }),
        )
        .await?;
        return Ok(());
    }
    let x = action.get("x").and_then(Value::as_f64).unwrap_or(0.0);
    let y = action.get("y").and_then(Value::as_f64).unwrap_or(0.0);
    let (ev_type, button, click_cnt, buttons) = match act {
        "mousedown" => ("mousePressed", "left", 1, 1),
        "mouseup" => ("mouseReleased", "left", 1, 0),
        _ => ("mouseMoved", "none", 0, 1),
    };
    cdp.call_page(
        target,
        "Input.dispatchMouseEvent",
        json!({
            "type": ev_type,
            "x": x,
            "y": y,
            "button": button,
            "clickCount": click_cnt,
            "buttons": buttons,
        }),
    )
    .await?;
    Ok(())
}

/// Dispatches text insertion or server-side `VaultStore` credential typing over CDP.
pub async fn dispatch_text_or_credential(
    cdp: &CdpController,
    target: &CdpPageTarget,
    act: &str,
    action: &Value,
    vault_username: Option<&str>,
    vault_password: Option<&str>,
) -> Result<Option<String>, String> {
    match act {
        "text" => {
            let txt = action.get("text").and_then(Value::as_str).unwrap_or("");
            if !txt.is_empty() {
                cdp.call_page(target, "Input.insertText", json!({ "text": txt }))
                    .await?;
            }
            Ok(None)
        }
        "vault_user" => match vault_username {
            Some(u) => {
                cdp.call_page(target, "Input.insertText", json!({ "text": u }))
                    .await?;
                Ok(Some("🔑 Vault ID 입력 완료".to_string()))
            }
            None => Ok(Some("저장된 Vault ID가 없습니다.".to_string())),
        },
        "vault_pass" => match vault_password {
            Some(p) => {
                cdp.call_page(target, "Input.insertText", json!({ "text": p }))
                    .await?;
                Ok(Some("🔒 Vault PW 입력 완료".to_string()))
            }
            None => Ok(Some("저장된 Vault PW가 없습니다.".to_string())),
        },
        _ => Ok(None),
    }
}

/// Dispatches navigation (`goto`, `reload`, `back`) or special keyboard keys over CDP.
pub async fn dispatch_nav_or_key(
    cdp: &CdpController,
    target: &CdpPageTarget,
    act: &str,
    action: &Value,
) -> Result<Option<String>, String> {
    match act {
        "key" => {
            let k = action.get("key").and_then(Value::as_str).unwrap_or("Enter");
            dispatch_special_key(cdp, target, k).await?;
            Ok(None)
        }
        "goto" => {
            let raw_url = action.get("url").and_then(Value::as_str).unwrap_or("");
            let safe_url = validate_http_url(raw_url)?;
            cdp.call_page(target, "Page.navigate", json!({ "url": safe_url }))
                .await?;
            Ok(None)
        }
        "reload" => {
            cdp.call_page(target, "Page.reload", json!({})).await?;
            Ok(Some("↻ 페이지 새로고침".to_string()))
        }
        "back" => {
            let _ = cdp
                .call_page(
                    target,
                    "Runtime.evaluate",
                    json!({ "expression": "window.history.back();" }),
                )
                .await;
            Ok(None)
        }
        _ => Ok(None),
    }
}

async fn dispatch_special_key(
    cdp: &CdpController,
    target: &CdpPageTarget,
    key: &str,
) -> Result<(), String> {
    let (vk, code, text_opt) = match key {
        "Enter" => (13, "Enter", Some("\r")),
        "Backspace" => (8, "Backspace", None),
        "Tab" => (9, "Tab", None),
        "Escape" => (27, "Escape", None),
        _ => (0, key, None),
    };
    cdp.call_page(
        target,
        "Input.dispatchKeyEvent",
        json!({
            "type": "rawKeyDown",
            "key": key,
            "code": code,
            "windowsVirtualKeyCode": vk,
            "nativeVirtualKeyCode": vk,
        }),
    )
    .await?;
    if let Some(ch) = text_opt {
        cdp.call_page(
            target,
            "Input.dispatchKeyEvent",
            json!({ "type": "char", "text": ch, "unmodifiedText": ch }),
        )
        .await?;
    }
    cdp.call_page(
        target,
        "Input.dispatchKeyEvent",
        json!({
            "type": "keyUp",
            "key": key,
            "code": code,
            "windowsVirtualKeyCode": vk,
            "nativeVirtualKeyCode": vk,
        }),
    )
    .await?;
    Ok(())
}

/// Reads selected text, visible code boxes (`[data-code-box]`, `code`, `input[readonly]`), or remote clipboard over CDP.
pub async fn extract_copyable_text(
    cdp: &CdpController,
    target: &CdpPageTarget,
) -> Result<Option<String>, String> {
    let expr = r#"(async () => {
      const sel = window.getSelection ? String(window.getSelection()).trim() : '';
      if (sel) return sel;
      const box = document.querySelector('[data-code-box], input[readonly], textarea[readonly], pre code, code');
      if (box) {
        const v = String(box.value ?? box.textContent ?? '').trim();
        if (v) return v;
      }
      try {
        if (navigator.clipboard && navigator.clipboard.readText) {
          const c = String(await navigator.clipboard.readText()).trim();
          if (c) return c;
        }
      } catch (_) {}
      return '';
    })()"#;
    let res = cdp
        .call_page(
            target,
            "Runtime.evaluate",
            json!({ "expression": expr, "awaitPromise": true, "returnByValue": true }),
        )
        .await?;
    let val = res
        .get("result")
        .and_then(|r| r.get("value"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(val)
}

const AISTUDIO_KEY_EXPR: &str = r#"(() => {
  const el = document.querySelector('[data-gemini-key]');
  if (el) {
    const k = String(el.textContent || el.value || '').trim();
    if (/^AIza[0-9A-Za-z_-]{35}$/.test(k)) return k;
  }
  for (const inp of document.querySelectorAll('input')) {
    const m = String(inp.value || '').trim().match(/\b(AIza[0-9A-Za-z_-]{35})\b/);
    if (m) return m[1];
  }
  const m = String(document.body?.innerText || '').match(/\b(AIza[0-9A-Za-z_-]{35})\b/);
  return m ? m[1] : '';
})()"#;

const OAUTH_CODE_EXPR: &str = r#"(() => {
  try {
    const q = new URL(location.href).searchParams.get('code');
    if (q && q.trim().length >= 8) return q.trim();
  } catch (_) {}
  const agy = document.querySelector('[data-code-box]');
  if (agy) {
    const v = String(agy.textContent || '').replace(/\s+/g, '');
    if (v.length >= 12) return v;
  }
  const el = document.querySelector('input[readonly], pre code, code');
  if (el) {
    const v = String(el.value ?? el.textContent ?? '').trim();
    if (v.length >= 12 && !/\s/.test(v)) return v;
  }
  return '';
})()"#;

/// Probes an active OAuth callback or AI Studio key page for an authorization code or `AIza...` API key.
pub async fn probe_oauth_callback_code(
    cdp: &CdpController,
    target: &CdpPageTarget,
) -> Option<String> {
    let u = target.url.as_str();
    let is_aistudio = u.contains("aistudio.google.com") || u.contains("ai.google.dev") || u.contains("apikey");
    if !is_aistudio && !u.contains("oauth-callback") && !u.contains("/oauth/code/callback") && !u.contains("code=") {
        return None;
    }
    let expr = if is_aistudio { AISTUDIO_KEY_EXPR } else { OAUTH_CODE_EXPR };
    let res = cdp
        .call_page(
            target,
            "Runtime.evaluate",
            json!({ "expression": expr, "returnByValue": true }),
        )
        .await
        .ok()?;
    res.get("result")
        .and_then(|r| r.get("value"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}
