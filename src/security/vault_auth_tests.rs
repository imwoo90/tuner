//! # Unit & Live Loopback Integration Tests for `vault_auth` (`vault_auth_tests.rs`)

use super::vault::{current_unix_secs, VaultEntry, VaultStore};
use super::vault_auth::{
    browser::BrowserRuntime,
    cdp::{cdp_call_ws, list_page_targets, CdpController},
    cdp_state::{send_graceful_browser_close, validate_http_url},
    canonical_storage_slug, parse_vault_auth_args,
    proxy::{
        build_ipc_router, has_active_auth_sessions, load_tunnel_endpoint, publish_tunnel_endpoint,
        RegisterAuthRouteReq, TunnelEndpointInfo, UnregisterAuthRouteReq,
    },
    resolve_auth_target,
    server::{await_session_completion, spawn_local_auth_server, AuthServerState, SessionOutcome},
    DEFAULT_AUTH_TTL_SECS,
};
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tower::ServiceExt;

#[test]
fn test_vault_auth_options_and_slug_resolution() {
    assert_eq!(canonical_storage_slug("https-jlcpcb.com"), "jlcpcb");
    assert_eq!(canonical_storage_slug("https-www.lcsc.com"), "lcsc");
    assert_eq!(canonical_storage_slug("aliexpress.com"), "aliexpress");
    assert_eq!(canonical_storage_slug("devicemart.co.kr"), "devicemart");
    assert_eq!(canonical_storage_slug("wimzzang@gmail.com"), "google");
    assert_eq!(canonical_storage_slug("holtlab.io@gmail.com"), "holtlab");

    assert!(validate_http_url("file:///etc/passwd").is_err());
    assert!(validate_http_url("javascript:alert(1)").is_err());
    assert!(validate_http_url("https://accounts.google.com/").is_ok());

    let args: Vec<String> = vec![
        "https-jlcpcb.com",
        "--ttl",
        "900",
        "--url",
        "https://passport.jlcpcb.com/login",
        "--no-tunnel",
        "--profile",
        "default",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let parsed = parse_vault_auth_args(&args).unwrap();
    assert_eq!(parsed.site_key, "https-jlcpcb.com");
    assert_eq!(parsed.ttl_secs, 900);
    assert!(parsed.no_tunnel);
    assert_eq!(
        parsed.start_url.as_deref(),
        Some("https://passport.jlcpcb.com/login")
    );

    let bad_args = vec!["aliexpress.com".to_string(), "--url".to_string(), "file:///etc/shadow".to_string()];
    assert!(parse_vault_auth_args(&bad_args).is_err());
}

#[test]
fn test_resolve_auth_target_with_and_without_vault_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let vault_dir = tmp.path().join("vault");
    let store = VaultStore::new(&vault_dir);
    store
        .upsert_entry(VaultEntry {
            site_key: "https-jlcpcb.com".to_string(),
            title: "JLCPCB".to_string(),
            url: "https://jlcpcb.com/order".to_string(),
            username: "wim@example.com".to_string(),
            password: "TopSecretValue!77".to_string(),
            updated_at_secs: 0,
        })
        .unwrap();

    // Lookup by canonical alias ("jlcpcb") still resolves the "https-jlcpcb.com" VaultEntry
    let opts = parse_vault_auth_args(&["jlcpcb".to_string()]).unwrap();
    assert_eq!(opts.ttl_secs, DEFAULT_AUTH_TTL_SECS);
    let target = resolve_auth_target(&vault_dir, &opts).unwrap();
    assert_eq!(target.slug, "jlcpcb");
    assert!(target.context_dir.ends_with("chromium_jlcpcb_ctx"));
    assert!(target.state_path.ends_with("jlcpcb_storage_state.json"));
    assert_eq!(target.start_url, "https://jlcpcb.com/order");
    assert_eq!(target.entry.unwrap().password, "TopSecretValue!77");

    // Passwordless OAuth site ("google") works cleanly even without a VaultEntry
    let g_opts = parse_vault_auth_args(&["google".to_string()]).unwrap();
    let g_target = resolve_auth_target(&vault_dir, &g_opts).unwrap();
    assert_eq!(g_target.slug, "google");
    assert!(g_target.entry.is_none());
    assert_eq!(g_target.start_url, "https://accounts.google.com/");
}

#[tokio::test]
async fn test_warm_tunnel_proxy_ipc_security_and_active_guard() {
    let tmp = tempfile::tempdir().unwrap();
    let vault_dir = tmp.path().join("vault");
    publish_tunnel_endpoint(&vault_dir, 8743).await;

    let ep = load_tunnel_endpoint(&vault_dir).unwrap();
    assert_eq!(ep.port, 8743);
    assert!(ep.ipc_port > 0);
    assert_eq!(ep.pid, std::process::id());
    assert_eq!(ep.ipc_secret.len(), 64);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(vault_dir.join("tunnel_endpoint.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    // Stale dead-worker PID must be rejected by load_tunnel_endpoint
    let stale_dir = tmp.path().join("stale_vault");
    std::fs::create_dir_all(&stale_dir).unwrap();
    let stale_ep = TunnelEndpointInfo {
        port: 8743,
        ipc_port: ep.ipc_port,
        pid: 999_999_999,
        ipc_secret: ep.ipc_secret.clone(),
    };
    std::fs::write(
        stale_dir.join("tunnel_endpoint.json"),
        serde_json::to_string(&stale_ep).unwrap(),
    )
    .unwrap();
    assert!(load_tunnel_endpoint(&stale_dir).is_none());

    let public_app = crate::telegram::review_server::build_review_router();
    let ipc_app = build_ipc_router();
    let token = hex::encode([0xABu8; 32]);

    // Spawn a genuine local HITL auth server for upstream ping verification
    let cdp = Arc::new(CdpController::new(19998, None, None));
    let auth_state = AuthServerState::new(
        token.clone(),
        "jlcpcb".to_string(),
        current_unix_secs() + 600,
        19998,
        "ws://127.0.0.1:19998/devtools/browser/mock".to_string(),
        tmp.path().join("jlcpcb_storage_state.json"),
        cdp,
    );
    let (genuine_hitl_port, srv_handle) = spawn_local_auth_server(auth_state).await.unwrap();
    let non_hitl_port = spawn_mock_target_site().await;

    let reg_payload = RegisterAuthRouteReq {
        token: token.clone(),
        local_port: genuine_hitl_port,
        expires_at_secs: current_unix_secs() + 600,
        skip_quick_tunnel: true,
    };

    // 0. Public :8743 router MUST NOT expose /internal/auth/register (404 Not Found)
    let res_pub_404 = public_app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/auth/register")
                .header("content-type", "application/json")
                .header("x-tuner-ipc-secret", &ep.ipc_secret)
                .body(Body::from(serde_json::to_string(&reg_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_pub_404.status(), StatusCode::NOT_FOUND);

    // 1. Wrong IPC secret on loopback IPC router -> 403 Forbidden
    let res_forbid = ipc_app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/auth/register")
                .header("content-type", "application/json")
                .header("x-tuner-ipc-secret", "wrong-secret")
                .body(Body::from(serde_json::to_string(&reg_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_forbid.status(), StatusCode::FORBIDDEN);

    // 2. SSRF attempt pointing local_port at a non-HITL service -> 400 Bad Request
    let ssrf_payload = RegisterAuthRouteReq {
        token: token.clone(),
        local_port: non_hitl_port,
        expires_at_secs: current_unix_secs() + 600,
        skip_quick_tunnel: true,
    };
    let res_ssrf = ipc_app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/auth/register")
                .header("content-type", "application/json")
                .header("x-tuner-ipc-secret", &ep.ipc_secret)
                .body(Body::from(serde_json::to_string(&ssrf_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_ssrf.status(), StatusCode::BAD_REQUEST);

    // 3. Genuine HITL server + valid IPC secret -> 200 OK & proxy page works
    let res_ok = ipc_app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/auth/register")
                .header("content-type", "application/json")
                .header("x-tuner-ipc-secret", &ep.ipc_secret)
                .body(Body::from(serde_json::to_string(&reg_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_ok.status(), StatusCode::OK);
    assert!(has_active_auth_sessions().await);

    let res_viewer = public_app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/auth/{token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_viewer.status(), StatusCode::OK);

    // 4. Unregister clears active auth session
    let unreg = UnregisterAuthRouteReq { token };
    let res_unreg = ipc_app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/auth/unregister")
                .header("content-type", "application/json")
                .header("x-tuner-ipc-secret", &ep.ipc_secret)
                .body(Body::from(serde_json::to_string(&unreg).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_unreg.status(), StatusCode::OK);
    assert!(!has_active_auth_sessions().await);
    srv_handle.abort();
}

async fn spawn_mock_target_site() -> u16 {
    let page_html = r#"<!DOCTYPE html><html><body>
        <input id="u" type="text" autofocus />
        <input id="p" type="password" />
        <script>localStorage.setItem('hitl_ls_key', 'hitl_ls_val');</script>
    </body></html>"#;
    let app = Router::new().route(
        "/login",
        get(move || async move {
            (
                [(header::SET_COOKIE, "hitl_session=verified_cookie_123; Path=/")],
                axum::response::Html(page_html),
            )
                .into_response()
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    port
}

async fn drive_mock_ws_viewer(ws_url: &str, secret_pw: &str) {
    let (mut ws, _) = connect_async(ws_url).await.unwrap();
    let mut got_jpeg = false;
    for _ in 0..20 {
        if let Some(Ok(Message::Binary(bytes))) = ws.next().await
            && bytes.len() > 100
            && bytes[0] == 0xFF
            && bytes[1] == 0xD8
        {
            got_jpeg = true;
            break;
        }
    }
    assert!(got_jpeg, "Expected JPEG binary frame from CDP streamer");

    // Forbidden file:// navigation must never crash or expose local files
    ws.send(Message::Text(
        json!({"type": "goto", "url": "file:///etc/passwd"}).to_string(),
    ))
    .await
    .unwrap();

    // Type Vault username, Tab to password, type Vault password, and save!
    for act in [
        json!({"type": "vault_user"}),
        json!({"type": "key", "key": "Tab"}),
        json!({"type": "vault_pass"}),
        json!({"type": "save_and_exit"}),
    ] {
        ws.send(Message::Text(act.to_string())).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    }

    let mut saw_done = false;
    while let Some(Ok(msg)) = ws.next().await {
        if let Message::Text(txt) = msg {
            assert!(
                !txt.contains(secret_pw),
                "Plaintext Vault password leaked over WebSocket: {txt}"
            );
            if txt.contains("\"done\":true") {
                saw_done = true;
                break;
            }
        }
    }
    assert!(saw_done, "Expected save_and_exit completion notice");
}

#[tokio::test]
async fn test_live_xvfb_cdp_vault_typing_storage_state_and_cleanup() {
    let site_port = spawn_mock_target_site().await;
    let tmp = tempfile::tempdir().unwrap();
    let ctx_dir = tmp.path().join("chromium_mock_ctx");
    let state_path = tmp.path().join("mock_storage_state.json");
    let secret_pw = "UltraSecretVaultPw#2026";

    let mut browser = BrowserRuntime::launch(&ctx_dir, "about:blank").await.unwrap();
    let cdp = Arc::new(CdpController::new(
        browser.cdp_port,
        Some("wim_user".to_string()),
        Some(secret_pw.to_string()),
    ));
    cdp.navigate_initial(&format!("http://127.0.0.1:{site_port}/login"))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let token = hex::encode([0xCDu8; 32]);
    let srv_state = AuthServerState::new(
        token.clone(),
        "mock".to_string(),
        current_unix_secs() + 120,
        browser.cdp_port,
        browser.browser_ws_url.clone(),
        state_path.clone(),
        cdp,
    );
    let (auth_port, srv_handle) = spawn_local_auth_server(srv_state.clone()).await.unwrap();

    // Verify viewer HTML never embeds plaintext password
    let html = reqwest::get(format!("http://127.0.0.1:{auth_port}/auth/{token}"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!html.contains(secret_pw));
    assert!(html.contains("\"has_password\":true"));

    let ws_url = format!("ws://127.0.0.1:{auth_port}/auth/{token}/ws");
    let client_task = tokio::spawn(async move {
        drive_mock_ws_viewer(&ws_url, secret_pw).await;
    });

    let outcome = await_session_completion(&srv_state, 20).await;
    client_task.await.unwrap();

    // Verify Vault credentials actually typed into DOM inputs via CDP
    let pages = list_page_targets(browser.cdp_port).await;
    let dom_vals = cdp_call_ws(
        &pages[0].ws_url,
        "Runtime.evaluate",
        json!({"expression": "document.getElementById('u').value + '|' + document.getElementById('p').value", "returnByValue": true}),
    )
    .await
    .unwrap();
    assert_eq!(
        dom_vals["result"]["value"].as_str(),
        Some("wim_user|UltraSecretVaultPw#2026")
    );

    match outcome {
        SessionOutcome::SavedByUser(sum) => assert!(sum.cookie_count >= 1),
        SessionOutcome::Aborted { reason, .. } => panic!("Unexpected abort: {reason}"),
    }

    srv_handle.abort();
    send_graceful_browser_close(&browser.browser_ws_url).await;
    browser.shutdown().await;

    // Verify exported Playwright storage_state.json schema & 0600 permissions
    let saved_json: Value =
        serde_json::from_str(&std::fs::read_to_string(&state_path).unwrap()).unwrap();
    assert!(saved_json["cookies"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["name"] == "hitl_session" && c["value"] == "verified_cookie_123"));
    assert!(saved_json["origins"]
        .as_array()
        .unwrap()
        .iter()
        .any(|o| o["localStorage"][0]["name"] == "hitl_ls_key"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&state_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
