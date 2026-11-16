//! # Human-in-the-Loop (HITL) Remote Browser Handoff (`vault_auth/mod.rs`)
//!
//! ## Overview
//! Implements `tuner --vault auth <site_key> [--url <url>] [--ttl <secs>]`, streaming a headed
//! Chrome persistent Vault profile (`chromium_<slug>_ctx`) under `Xvfb` over a 256-bit token-gated
//! Quick Tunnel so humans solve CAPTCHA / 2FA / OAuth prompts on mobile and reactively resume agents.
//!
//! ## Submodules
//! - [`browser`] (`browser.rs`): `Xvfb -displayfd 1` and headed Chrome persistent profile launcher.
//! - [`browser_env`] (`browser_env.rs`): Browser executable discovery and profile lock hygiene.
//! - [`cdp`] (`cdp.rs`): Pure Rust CDP WebSocket controller for MRU tab tracking and screenshots.
//! - [`cdp_input`] (`cdp_input.rs`): CDP pointer, wheel, keyboard, and Zero-Knowledge Vault input helpers.
//! - [`cdp_state`] (`cdp_state.rs`): Cookie seeding and Playwright `storage_state.json` (`0600`) exporter.
//! - [`options`] (`options.rs`): CLI flag parser, canonical storage slug mapper, and Vault target resolver.
//! - [`proxy`] (`proxy.rs`): Loopback `127.0.0.1` IPC registry, PID check, and SSRF ping guard.
//! - [`proxy_relay`] (`proxy_relay.rs`): Public `:8743` HTTP/WebSocket reverse proxy routes.
//! - [`server`] (`server.rs`): Loopback Axum UI/WebSocket server, single-viewer lock, and signal/TTL handler.
//! - [`tunnel`] (`tunnel.rs`): Warm `:8743` proxy registration with standalone `cloudflared` fallback.
//!
//! ## Search Tags
//! #hitl-browser-auth, #vault-auth, #web-in-web, #captcha-handoff

pub mod browser;
pub mod browser_env;
pub mod cdp;
pub mod cdp_input;
pub mod cdp_state;
pub mod options;
pub mod proxy;
pub mod proxy_relay;
pub mod server;
pub mod tunnel;

pub use options::{
    canonical_storage_slug, parse_vault_auth_args, resolve_auth_target, ResolvedAuthTarget,
    VaultAuthOptions, DEFAULT_AUTH_TTL_SECS,
};

use crate::security::vault::current_unix_secs;
use crate::workspace::paths::resolve_paths;
use browser::BrowserRuntime;
use cdp::CdpController;
use cdp_state::{export_storage_state, seed_existing_storage_state, send_graceful_browser_close};
use serde_json::json;
use server::{spawn_local_auth_server, AuthServerState, SessionOutcome};
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use tunnel::establish_auth_tunnel;

/// Blocks until the user saves, TTL expires, or `SIGINT`/`SIGTERM` arrives, auto-flushing state on exit.
pub async fn await_session_completion(
    state: &Arc<AuthServerState>,
    ttl_secs: u64,
) -> SessionOutcome {
    let abort_reason = tokio::select! {
        _ = state.done_notify.notified() => None,
        _ = tokio::time::sleep(Duration::from_secs(ttl_secs)) => Some("timeout".to_string()),
        _ = wait_termination_signal() => Some("signal".to_string()),
    };
    if let Some(summary) = state.done_summary.lock().await.take() {
        return SessionOutcome::SavedByUser(summary);
    }
    let flushed = export_storage_state(state.cdp_port, &state.browser_ws_url, &state.state_path)
        .await
        .ok();
    SessionOutcome::Aborted {
        reason: abort_reason.unwrap_or_else(|| "closed".to_string()),
        flushed,
    }
}

async fn wait_termination_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = sigterm.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

fn emit_session_banner(
    profile: &str,
    opts: &VaultAuthOptions,
    target: &ResolvedAuthTarget,
    public_url: &str,
) {
    println!("[REMOTE_AUTH_URL] {public_url}");
    println!(
        "[REMOTE_AUTH_INFO] {}",
        json!({
            "profile": profile,
            "site_key": opts.site_key,
            "slug": target.slug,
            "url": public_url,
            "ttl_secs": opts.ttl_secs,
            "vault_credentials_loaded": target.entry.is_some(),
            "context_dir": target.context_dir,
            "state_path": target.state_path,
        })
    );
    let _ = std::io::stdout().flush();
}

fn finalize_outcome(outcome: SessionOutcome, target: &ResolvedAuthTarget) -> Result<(), String> {
    match outcome {
        SessionOutcome::SavedByUser(summary) => {
            println!(
                "[REMOTE_AUTH_SAVED] {}",
                json!({
                    "status": "saved",
                    "slug": target.slug,
                    "cookie_count": summary.cookie_count,
                    "origin_count": summary.origin_count,
                    "final_url": summary.final_url,
                    "context_dir": target.context_dir,
                    "state_path": target.state_path,
                })
            );
            Ok(())
        }
        SessionOutcome::Aborted { reason, flushed } => {
            let cookies = flushed.as_ref().map(|f| f.cookie_count).unwrap_or(0);
            Err(format!(
                "Remote auth ended ({reason}); auto-flushed {cookies} cookie(s) to {}",
                target.state_path.display()
            ))
        }
    }
}

async fn launch_configured_browser(
    target: &ResolvedAuthTarget,
) -> Result<(BrowserRuntime, Arc<CdpController>), String> {
    let browser = BrowserRuntime::launch(&target.context_dir, "about:blank").await?;
    seed_existing_storage_state(&browser.browser_ws_url, &target.state_path).await;
    let cdp = Arc::new(CdpController::new(
        browser.cdp_port,
        target.entry.as_ref().map(|e| e.username.clone()),
        target.entry.as_ref().map(|e| e.password.clone()),
    ));
    let _ = cdp.navigate_initial(&target.start_url).await;
    Ok((browser, cdp))
}

/// Executes `tuner --vault auth ...` end-to-end.
pub async fn run_vault_auth_cli(profile: &str, tail_args: &[String]) -> Result<(), String> {
    let opts = parse_vault_auth_args(tail_args)?;
    let vault_dir = resolve_paths(None, None, None, Some(profile.to_string())).vault_dir();
    let target = resolve_auth_target(&vault_dir, &opts)?;
    let (mut browser, cdp) = launch_configured_browser(&target).await?;

    let token = hex::encode(rand::random::<[u8; 32]>());
    let expires_at = current_unix_secs().saturating_add(opts.ttl_secs);
    let srv_state = AuthServerState::new(
        token.clone(),
        target.slug.clone(),
        expires_at,
        browser.cdp_port,
        browser.browser_ws_url.clone(),
        target.state_path.clone(),
        cdp,
    );

    let (local_port, srv_handle) = match spawn_local_auth_server(srv_state.clone()).await {
        Ok(v) => v,
        Err(e) => {
            browser.shutdown().await;
            return Err(e);
        }
    };

    let (public_url, mut active_tunnel) = match establish_auth_tunnel(
        &vault_dir,
        local_port,
        &token,
        opts.ttl_secs,
        opts.no_tunnel,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            srv_handle.abort();
            browser.shutdown().await;
            return Err(e);
        }
    };

    emit_session_banner(profile, &opts, &target, &public_url);
    let outcome = await_session_completion(&srv_state, opts.ttl_secs).await;

    active_tunnel.teardown().await;
    srv_handle.abort();
    send_graceful_browser_close(&browser.browser_ws_url).await;
    browser.shutdown().await;

    finalize_outcome(outcome, &target)
}
