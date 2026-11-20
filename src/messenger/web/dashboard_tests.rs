//! # Web Dashboard Unit & Integration Tests (`dashboard_tests.rs`)
//!
//! ## Overview
//! Verifies authentication, Magic Links, PBKDF2 PINs, CSRF/Origin checks, IP lockout,
//! file sandboxing (including symlink escape rejection), sessions, multipart uploads,
//! `AskQuestion` flows, `/stop`/`/new`, recursive subagent tree inspection, cron toggle,
//! zero-knowledge vault masking, and CLI argument parsing.
//!
//! ## Search Tags
//! #dashboard-tests, #auth-security-tests, #file-sandbox-tests

use super::auth::WebAuthStore;
use super::auth_http::extract_session_cookie;
use super::ws::is_valid_ws_origin;
use super::cli_dash::{
    load_dashboard_tunnel_url, parse_dashboard_cli_args, publish_dashboard_tunnel_url,
};
use super::events::{WebEvent, WebEventHub};
use super::subagent_tree::collect_agent_tree_nodes;
use super::{build_router_with_ctx, WebAppCtx};
use crate::bus::bus::MessageBus;
use crate::cli::antigravity::session::AskState;
use crate::cli::antigravity::AntigravityCli;
use crate::cli::AskQuestionData;
use crate::config::CliConfig;
use crate::cron::manager::{CronJob, CronManager};
use crate::messenger::telegram::history::log_telegram_message;
use crate::messenger::telegram::TopicNameCache;
use crate::session::key::SessionKey;
use crate::session::manager::SessionManager;
use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode};
use axum::Router;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

fn build_test_ctx(root: &std::path::Path) -> Arc<WebAppCtx> {
    let profile_home = root.join("profiles").join("default");
    let tuner_home = root.to_path_buf();
    let vault_dir = profile_home.join("vault");
    let working_dir = profile_home.join("workspace");
    let cfg_dir = profile_home.join("config");
    std::fs::create_dir_all(&vault_dir).unwrap();
    std::fs::create_dir_all(&working_dir).unwrap();
    std::fs::create_dir_all(&cfg_dir).unwrap();

    let cfg = CliConfig {
        provider: "antigravity".to_string(),
        model: Some("gemini-3.5-pro".to_string()),
        working_dir: working_dir.clone(),
        allowed_user_ids: vec![12345],
        ..Default::default()
    };
    let sessions = Arc::new(SessionManager::new(
        profile_home.join("sessions.json"),
        120,
        4,
        false,
        "UTC".to_string(),
        None,
    ));
    let cli = Arc::new(AntigravityCli::new(cfg.clone()));
    let bus = Arc::new(MessageBus::with_lock_pool(sessions.lock_pool.clone()));
    let cron_manager = Arc::new(CronManager::new(profile_home.join("cron_jobs.json")));
    Arc::new(WebAppCtx {
        profile: "default".to_string(),
        tuner_home,
        vault_dir: vault_dir.clone(),
        working_dir,
        config: Arc::new(cfg),
        sessions,
        cli,
        cron_manager,
        topic_cache: Arc::new(TopicNameCache::new()),
        bus,
        bot: RwLock::new(None),
        events: Arc::new(WebEventHub::new()),
        auth: Arc::new(WebAuthStore::new(vault_dir)),
    })
}

async fn body_json(res: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn pct_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

async fn login_via_magic(app: &Router, ctx: &Arc<WebAppCtx>) -> (String, String) {
    let tok = ctx.auth.issue_magic_token().await.unwrap();
    assert_eq!(tok.len(), 64);
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/dashboard/magic/{tok}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(cookie.contains("tuner_dash_sid="));
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Lax"));
    let sid_kv = cookie.split(';').next().unwrap().to_string();

    // Replay of the same magic token must be rejected
    let replay = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/dashboard/magic/{tok}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);

    let me = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dashboard/api/auth/me")
                .header(header::COOKIE, &sid_kv)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);
    let me_json = body_json(me).await;
    assert!(me_json["ready_providers"].is_array());
    let csrf = me_json["csrf_token"].as_str().unwrap().to_string();
    (sid_kv, csrf)
}

#[tokio::test]
async fn test_auth_magic_pin_csrf_rate_limit_and_revocation() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = build_test_ctx(dir.path());
    let app = build_router_with_ctx(ctx.clone());

    // Unauthenticated HTML and API checks
    let html_unauth = app
        .clone()
        .oneshot(Request::builder().uri("/dashboard").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(html_unauth.status(), StatusCode::SEE_OTHER);
    assert_eq!(html_unauth.headers().get(header::LOCATION).unwrap(), "/dashboard/login");

    let api_unauth = app
        .clone()
        .oneshot(Request::builder().uri("/dashboard/api/sessions").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(api_unauth.status(), StatusCode::UNAUTHORIZED);

    // Magic login + replay rejection + 0600 permissions
    let (cookie, csrf) = login_via_magic(&app, &ctx).await;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(ctx.vault_dir.join("dashboard_auth.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    // Mutating request without CSRF -> 403; wrong CSRF -> 403
    let no_csrf = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/sessions")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"topic_name":"Missing Csrf"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), StatusCode::FORBIDDEN);

    let bad_csrf = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/sessions")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", "deadbeef")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"topic_name":"Bad Csrf"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_csrf.status(), StatusCode::FORBIDDEN);

    // Set PIN via authenticated mutation, then login with PIN
    let set_pin_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/auth/pin")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"pin":"tuner-secret-2026"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(set_pin_res.status(), StatusCode::OK);

    let ok_login = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"pin":"tuner-secret-2026"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok_login.status(), StatusCode::OK);
    let login_cookie = ok_login
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    // 5 failed PIN attempts trigger 429 lockout
    for _ in 0..5 {
        let wrong = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/dashboard/api/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"pin":"wrong-pin"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    }
    let locked = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"pin":"tuner-secret-2026"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(locked.status(), StatusCode::TOO_MANY_REQUESTS);

    // Revoke all clears all existing sessions
    let rev = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/auth/revoke_all")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rev.status(), StatusCode::OK);

    let after_rev = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dashboard/api/sessions")
                .header(header::COOKIE, &login_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(after_rev.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn test_ws_origin_and_cookie_extraction() {
    let mut h = HeaderMap::new();
    h.insert(header::HOST, HeaderValue::from_static("127.0.0.1:8743"));
    h.insert(header::ORIGIN, HeaderValue::from_static("http://127.0.0.1:8743"));
    assert!(is_valid_ws_origin(&h));

    h.insert(header::ORIGIN, HeaderValue::from_static("https://evil.attacker.tld"));
    assert!(!is_valid_ws_origin(&h));

    h.insert(header::COOKIE, HeaderValue::from_static("foo=bar; tuner_dash_sid=abc123; baz=1"));
    assert_eq!(extract_session_cookie(&h).as_deref(), Some("abc123"));
}

#[tokio::test]
async fn test_file_sandbox_and_symlink_escape_denial() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = build_test_ctx(dir.path());
    let app = build_router_with_ctx(ctx.clone());
    let (cookie, _) = login_via_magic(&app, &ctx).await;

    let safe_file = ctx.working_dir.join("hello.rs");
    std::fs::write(&safe_file, "fn main() {}\n").unwrap();
    std::fs::write(ctx.tuner_home.join(".env"), "SECRET=supersecret\n").unwrap();
    std::fs::write(ctx.vault_dir.join("vault.enc"), "ciphertext").unwrap();
    std::fs::write(ctx.tuner_home.join("profiles/default/config/config.json"), "{}").unwrap();
    let ssh_dir = ctx.working_dir.join(".ssh");
    std::fs::create_dir_all(&ssh_dir).unwrap();
    std::fs::write(ssh_dir.join("id_rsa"), "KEY").unwrap();

    #[cfg(unix)]
    std::os::unix::fs::symlink(
        ctx.tuner_home.join(".env"),
        ctx.working_dir.join("evil_link.txt"),
    )
    .unwrap();

    let ok_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/dashboard/api/file?path={}", pct_encode(&safe_file.to_string_lossy())))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok_res.status(), StatusCode::OK);
    let ok_json = body_json(ok_res).await;
    assert_eq!(ok_json["language"], "rust");
    assert!(ok_json["content"].as_str().unwrap().contains("fn main()"));

    let raw_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/dashboard/api/file?path={}&raw=1", pct_encode(&safe_file.to_string_lossy())))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(raw_res.status(), StatusCode::OK);
    assert_eq!(raw_res.headers().get(header::X_CONTENT_TYPE_OPTIONS).unwrap(), "nosniff");

    let mut blocked_paths = vec![
        "/etc/passwd".to_string(),
        ctx.tuner_home.join(".env").to_string_lossy().to_string(),
        ctx.vault_dir.join("vault.enc").to_string_lossy().to_string(),
        ctx.vault_dir.join("dashboard_auth.json").to_string_lossy().to_string(),
        ctx.tuner_home.join("profiles/default/config/config.json").to_string_lossy().to_string(),
        ssh_dir.join("id_rsa").to_string_lossy().to_string(),
        format!("{}/../../../etc/passwd", ctx.working_dir.display()),
    ];
    #[cfg(unix)]
    blocked_paths.push(ctx.working_dir.join("evil_link.txt").to_string_lossy().to_string());

    for bp in blocked_paths {
        let r = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/dashboard/api/file?path={}", pct_encode(&bp)))
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "expected 403 for sensitive path {bp}");
    }
}

#[tokio::test]
async fn test_sessions_models_history_upload_ask_and_reset() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = build_test_ctx(dir.path());
    let app = build_router_with_ctx(ctx.clone());
    let (cookie, csrf) = login_via_magic(&app, &ctx).await;

    // Create monotonic web:0:1 and web:0:2 sessions
    let c1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/sessions")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"topic_name":"Alpha Web"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(c1.status(), StatusCode::CREATED);
    let s1 = body_json(c1).await;
    assert_eq!(s1["session_key"], "web:0:1");

    let c2 = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/sessions")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"topic_name":"Beta Claude","provider":"claude","model":"claude-sonnet-4-6","effort":"high"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(c2.status(), StatusCode::CREATED);
    let s2 = body_json(c2).await;
    assert_eq!(s2["session_key"], "web:0:2");
    assert_eq!(s2["provider"], "claude");

    // Seed a unified Telegram session and list both
    let tg_key = SessionKey::telegram(-100777, Some(42));
    let _ = ctx.sessions.get_or_init_session(&tg_key, "antigravity", "gemini-3.5-pro").await.unwrap();
    let list_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dashboard/api/sessions")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list_res.status(), StatusCode::OK);
    let list_json = body_json(list_res).await;
    assert_eq!(list_json["sessions"].as_array().unwrap().len(), 3);

    // Patch provider/model/effort/topic_name
    let patch_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri("/dashboard/api/sessions/config")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"session_key":"web:0:1","provider":"codex","model":"gpt-6-astra","effort":"xhigh","topic_name":"Alpha Renamed"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(patch_res.status(), StatusCode::OK);
    let p_json = body_json(patch_res).await;
    assert_eq!(p_json["provider"], "codex");
    assert_eq!(p_json["effort"], "xhigh");
    assert_eq!(p_json["topic_name"], "Alpha Renamed");

    // Models catalog check
    let mod_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dashboard/api/models?provider=claude")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(mod_res.status(), StatusCode::OK);
    assert!(!body_json(mod_res).await["models"].as_array().unwrap().is_empty());

    // Multipart upload attaches pending file
    let boundary = "----TUNERBOUNDARY999";
    let mp_body = format!(
        "--{b}\r\nContent-Disposition: form-data; name=\"session_key\"\r\n\r\nweb:0:1\r\n--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"spec.txt\"\r\nContent-Type: text/plain\r\n\r\nhello attachment\r\n--{b}--\r\n",
        b = boundary
    );
    let up_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/sessions/upload")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
                .body(Body::from(mp_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(up_res.status(), StatusCode::OK);
    let up_json = body_json(up_res).await;
    assert!(up_json["path"].as_str().unwrap().starts_with("telegram_files/web_"));

    // Seed provider session_id, transcript log, live delta buffer, and mock AskState
    let web1 = "web:0:1".parse::<SessionKey>().unwrap();
    ctx.sessions
        .mutate_session(&web1, None, |cur| {
            cur.set_session_id("codex", "mock-cx-1");
            true
        })
        .await
        .unwrap();
    log_telegram_message(&ctx.working_dir, "mock-cx-1", (Some(1), Some("Alpha Renamed")), "user", None, "Hi Codex", (true, None));
    ctx.events.publish_throttled_delta("web:0:1", "Streaming chunk...", Some("mock-cx-1".to_string()), 0).await;
    ctx.cli
        .sessions
        .set_ask_state(
            "mock-cx-1",
            AskState {
                msg_id: 0,
                questions: vec![AskQuestionData {
                    question: "Pick deployment target?".to_string(),
                    options: vec!["Staging".to_string(), "Production".to_string()],
                    is_multi_select: false,
                }],
                current_index: 0,
                answers: vec![String::new()],
                current_bitmap: String::new(),
                waiting_for_write_in: false,
            },
        )
        .await;

    let hist_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dashboard/api/sessions/history?session_key=web:0:1")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hist_res.status(), StatusCode::OK);
    let hist_json = body_json(hist_res).await;
    assert_eq!(hist_json["messages"].as_array().unwrap().len(), 1);
    assert_eq!(hist_json["live_stream"], "Streaming chunk...");
    assert_eq!(hist_json["active_ask"]["questions"][0]["question"], "Pick deployment target?");

    // Answer active AskQuestion
    let ask_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/sessions/ask")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"session_key":"web:0:1","action":"answer","selected_indices":[1]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ask_res.status(), StatusCode::OK);
    let ask_json = body_json(ask_res).await;
    assert_eq!(ask_json["resolved"], true);
    assert_eq!(ask_json["summary"], "Production");
    assert!(!ctx.cli.sessions.is_ask_active("mock-cx-1").await);

    // Stop & /new reset
    let stop_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/sessions/stop")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"session_key":"web:0:1"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stop_res.status(), StatusCode::OK);

    let new_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/sessions/new")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"session_key":"web:0:1"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(new_res.status(), StatusCode::OK);
    assert!(body_json(new_res).await["session_id"].as_str().unwrap().starts_with("mock-web-"));
}

#[tokio::test]
async fn test_subagent_tree_cron_vault_hitl_and_cli_args() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = build_test_ctx(dir.path());
    let app = build_router_with_ctx(ctx.clone());
    let (cookie, csrf) = login_via_magic(&app, &ctx).await;

    // Seed brain/.system_generated hierarchy
    let brain = dir.path().join("brain");
    let root_gen = brain.join("cid-root").join(".system_generated");
    std::fs::create_dir_all(root_gen.join("subagents")).unwrap();
    std::fs::create_dir_all(root_gen.join("swarm")).unwrap();
    std::fs::create_dir_all(root_gen.join("tasks")).unwrap();
    std::fs::create_dir_all(root_gen.join("logs")).unwrap();
    std::fs::write(
        root_gen.join("subagents/child1.json"),
        r#"{"conversationId":"cid-child","role":"Verifier","state":"SUBAGENT_STATE_ALIVE"}"#,
    )
    .unwrap();
    std::fs::write(
        root_gen.join("swarm/members.json"),
        r#"{"members":{"builder":{"name":"builder","model":"gemini-3.5","state":"working","conversation_id":"cid-child"}}}"#,
    )
    .unwrap();
    std::fs::write(root_gen.join("tasks/build.log"), "compiling...\nfinished ok\n").unwrap();
    std::fs::write(
        root_gen.join("logs/transcript.jsonl"),
        "{\"step_index\":7,\"type\":\"PLANNER_RESPONSE\",\"status\":\"RUNNING\",\"tool_calls\":[{\"name\":\"run_command\"}]}\n",
    )
    .unwrap();
    let child_gen = brain.join("cid-child").join(".system_generated/logs");
    std::fs::create_dir_all(&child_gen).unwrap();
    std::fs::write(child_gen.join("transcript.jsonl"), "{\"step_index\":1,\"type\":\"USER_INPUT\",\"status\":\"DONE\"}\n").unwrap();

    let tree = collect_agent_tree_nodes(&brain, "cid-root", 16);
    assert_eq!(tree.len(), 2);
    assert_eq!(tree[0].subagents[0].role, "Verifier");
    assert_eq!(tree[0].swarm_members[0].name, "builder");
    assert!(tree[0].tasks[0].tail.contains("finished ok"));
    assert_eq!(tree[0].latest_step.as_ref().unwrap().tools, vec!["run_command"]);

    // Seed Cron job and toggle via HTTP
    let job = CronJob::new(
        "job-nightly".to_string(),
        "Nightly Check".to_string(),
        "0 2 * * *".to_string(),
        "nightly".to_string(),
        "Check status".to_string(),
        12345,
        None,
    );
    ctx.cron_manager.save(&[job]).unwrap();
    let cron_list = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dashboard/api/cron")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cron_list.status(), StatusCode::OK);
    assert_eq!(body_json(cron_list).await["jobs"][0]["enabled"], true);

    let toggle = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/cron/job-nightly/toggle")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(toggle.status(), StatusCode::OK);
    assert_eq!(body_json(toggle).await["enabled"], false);

    // Zero-knowledge Vault CRUD: write-only password must never appear in GET response
    let v_up = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/vault/entries")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"site_key":"github","title":"GitHub","url":"https://github.com/login","username":"alice","password":"TOP-SECRET-VALUE-999"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(v_up.status(), StatusCode::OK);
    let up_bytes = to_bytes(v_up.into_body(), 1024 * 1024).await.unwrap();
    let up_txt = String::from_utf8_lossy(&up_bytes);
    assert!(!up_txt.contains("TOP-SECRET-VALUE-999"));
    assert!(up_txt.contains("github"));

    let v_get = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dashboard/api/vault/entries")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let get_bytes = to_bytes(v_get.into_body(), 1024 * 1024).await.unwrap();
    let get_txt = String::from_utf8_lossy(&get_bytes);
    assert!(!get_txt.contains("TOP-SECRET-VALUE-999"));
    assert!(!get_txt.contains("\"alice\""));
    assert!(get_txt.contains("ali***"));

    let v_del = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/dashboard/api/vault/entries/github")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(v_del.status(), StatusCode::OK);

    // HITL status & subagents endpoint
    let hitl = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dashboard/api/hitl")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hitl.status(), StatusCode::OK);
    assert_eq!(body_json(hitl).await["active"], false);

    let sub = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dashboard/api/subagents")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(sub.status(), StatusCode::OK);

    // EventHub live stream clear on TurnResult
    ctx.events
        .publish(WebEvent::TurnResult {
            session_key: "web:0:9".to_string(),
            session_id: Some("s9".to_string()),
            text: "Done".to_string(),
            is_error: false,
            files: Vec::new(),
            timestamp: "2026-11-17T00:00:00Z".to_string(),
        })
        .await;
    assert!(ctx.events.get_live_stream("web:0:9").await.is_none());

    // CLI args & tunnel discovery file
    let args = vec![
        "tuner".to_string(),
        "--dashboard".to_string(),
        "--profile".to_string(),
        "work".to_string(),
        "--set-pin".to_string(),
        "9876".to_string(),
    ];
    let (prof, pin) = parse_dashboard_cli_args(&args).unwrap();
    assert_eq!(prof, "work");
    assert_eq!(pin.as_deref(), Some("9876"));

    publish_dashboard_tunnel_url(&PathBuf::from(&ctx.vault_dir), Some("https://demo.trycloudflare.com"));
    assert_eq!(
        load_dashboard_tunnel_url(&ctx.vault_dir).as_deref(),
        Some("https://demo.trycloudflare.com")
    );
    publish_dashboard_tunnel_url(&ctx.vault_dir, None);
    assert!(load_dashboard_tunnel_url(&ctx.vault_dir).is_none());
}

#[tokio::test]
async fn test_round1_audit_regressions_sw_bus_ws_origin() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = build_test_ctx(dir.path());
    ctx.bus
        .register_transport(Arc::new(super::WebTransport::new(ctx.clone())));
    let app = build_router_with_ctx(ctx.clone());

    // 1. DEF-R1-01: /dashboard/sw.js must serve Service-Worker-Allowed: /dashboard
    let sw_res = app
        .clone()
        .oneshot(Request::builder().uri("/dashboard/sw.js").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(sw_res.status(), StatusCode::OK);
    assert_eq!(
        sw_res
            .headers()
            .get("service-worker-allowed")
            .and_then(|v| v.to_str().ok()),
        Some("/dashboard")
    );

    // 2. Preview bot User-Agent must NOT consume single-use Magic Link
    let tok = ctx.auth.issue_magic_token().await.unwrap();
    let bot_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/dashboard/magic/{tok}"))
                .header(header::USER_AGENT, "TelegramBot (like TwitterBot)")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bot_res.status(), StatusCode::NO_CONTENT);
    let human_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/dashboard/magic/{tok}"))
                .header(header::USER_AGENT, "Mozilla/5.0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(human_res.status(), StatusCode::SEE_OTHER);

    // 3. Default port normalization on WebSocket Origin vs Host
    let mut mismatch_hdrs = HeaderMap::new();
    mismatch_hdrs.insert(header::ORIGIN, HeaderValue::from_static("http://127.0.0.1:8743"));
    mismatch_hdrs.insert(header::HOST, HeaderValue::from_static("127.0.0.1"));
    assert!(!is_valid_ws_origin(&mismatch_hdrs));

    let mut ok_default_hdrs = HeaderMap::new();
    ok_default_hdrs.insert(header::ORIGIN, HeaderValue::from_static("https://demo.trycloudflare.com"));
    ok_default_hdrs.insert(header::HOST, HeaderValue::from_static("demo.trycloudflare.com:443"));
    assert!(is_valid_ws_origin(&ok_default_hdrs));

    // 4. DEF-R1-04: Web-Standalone fallback envelope stripped & routed to web:0:* session
    let _ = ctx
        .sessions
        .create_web_session(
            Some("Standalone Cron Topic".to_string()),
            "antigravity",
            "gemini-4-hydrogen",
            None,
        )
        .await
        .unwrap();
    let mut rx = ctx.events.subscribe();
    let mut env = crate::bus::adapters::from_cron_result(
        "Daily Brief",
        "CRON_STANDALONE_CLEAN_OK",
        "success",
        Some(999999),
        None,
        Some("tg"),
    );
    ctx.bus.submit(&mut env).await;
    let received = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    match received {
        WebEvent::TurnResult { session_key, text, is_error, .. } => {
            assert_eq!(session_key, "web:0:1");
            assert!(!is_error);
            assert!(text.contains("CRON_STANDALONE_CLEAN_OK"));
            assert!(!text.contains("Delivery fallback"));
        }
        other => panic!("Expected TurnResult, got {other:?}"),
    }
}

#[tokio::test]
async fn test_round2_open_ask_chat_routing_and_directive_guard() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = build_test_ctx(dir.path());
    let mut sess = ctx
        .sessions
        .create_web_session(Some("Ask Chat Route".into()), "antigravity", "gemini-4-hydrogen", None)
        .await
        .unwrap();
    let key = sess.session_key();
    sess.set_session_id("antigravity", "mock-ask-r2");
    let _ = ctx
        .sessions
        .mutate_session(&key, Some(&sess), |cur| {
            cur.set_session_id("antigravity", "mock-ask-r2");
            true
        })
        .await
        .unwrap();

    let questions = vec![
        AskQuestionData {
            question: "Pick step 1".into(),
            options: vec!["Alpha".into(), "Beta".into(), "Write-in".into()],
            is_multi_select: false,
        },
        AskQuestionData {
            question: "Pick step 2".into(),
            options: vec!["One".into(), "Two".into(), "Write-in".into()],
            is_multi_select: false,
        },
    ];
    ctx.cli
        .sessions
        .set_ask_state(
            "mock-ask-r2",
            AskState {
                msg_id: 0,
                questions,
                current_index: 0,
                answers: vec![String::new(), String::new()],
                current_bitmap: String::new(),
                waiting_for_write_in: false,
            },
        )
        .await;

    // 1. Leading @model + prompt must be rejected without clobbering active ask
    super::turn_runner::execute_web_turn(
        ctx.clone(),
        key.clone(),
        "@pro start clobbering turn".into(),
        vec![],
    )
    .await;
    assert!(ctx.cli.sessions.is_ask_active("mock-ask-r2").await);
    assert_eq!(ctx.cli.sessions.get_ask_state("mock-ask-r2").await.unwrap().current_index, 0);

    // 2. Plain chat input answers Step 1 and queues attachment for next turn
    super::turn_runner::execute_web_turn(
        ctx.clone(),
        key.clone(),
        "Beta".into(),
        vec!["/tmp/queued_next.txt".into()],
    )
    .await;
    assert!(ctx.cli.sessions.is_ask_active("mock-ask-r2").await);
    let st1 = ctx.cli.sessions.get_ask_state("mock-ask-r2").await.unwrap();
    assert_eq!(st1.current_index, 1);
    assert_eq!(st1.answers[0], "Beta");
    let reloaded = ctx.sessions.get_active(&key).await.unwrap().unwrap();
    assert_eq!(reloaded.pending_attachments, vec!["/tmp/queued_next.txt".to_string()]);

    // 3. Plain chat write-in answers Step 2 and resolves the open ask
    let mut rx = ctx.events.subscribe();
    super::turn_runner::execute_web_turn(ctx.clone(), key.clone(), "Custom Step 2".into(), vec![]).await;
    assert!(!ctx.cli.sessions.is_ask_active("mock-ask-r2").await);
    assert!(ctx.cli.sessions.get_ask_state("mock-ask-r2").await.is_none());
    let mut saw_resolved = false;
    while let Ok(ev) = rx.try_recv() {
        if matches!(ev, WebEvent::AskResolved { ref summary, .. } if summary == "Custom Step 2") {
            saw_resolved = true;
        }
    }
    assert!(saw_resolved);
}

#[tokio::test]
async fn test_round2_relative_file_preview_and_web_unicast_cron() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = build_test_ctx(dir.path());
    let sub = ctx.working_dir.join("docs");
    std::fs::create_dir_all(&sub).unwrap();
    let doc_path = sub.join("guide.md");
    std::fs::write(&doc_path, "# Guide\n").unwrap();

    let resolved = super::api_files::validate_preview_file(&ctx, "docs/guide.md").unwrap();
    assert_eq!(resolved, std::fs::canonicalize(&doc_path).unwrap());
    assert!(super::api_files::validate_preview_file(&ctx, "docs/../.env").is_err());

    let web_env = crate::bus::adapters::from_cron_result(
        "Web Topic Run",
        "ok",
        "success",
        None,
        Some(7),
        Some("web"),
    );
    assert_eq!(web_env.delivery, crate::bus::envelope::DeliveryMode::Unicast);

    let bcast_env = crate::bus::adapters::from_cron_result(
        "Global Broadcast",
        "ok",
        "success",
        None,
        None,
        Some("tg"),
    );
    assert_eq!(bcast_env.delivery, crate::bus::envelope::DeliveryMode::Broadcast);
}

#[tokio::test]
async fn test_round3_atk_t1_01_preamble_turn_result_preserves_open_ask() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = build_test_ctx(dir.path());
    let mut sess = ctx
        .sessions
        .create_web_session(Some("Preamble Ask".into()), "antigravity", "gemini-4-hydrogen", None)
        .await
        .unwrap();
    let key = sess.session_key();
    sess.set_session_id("antigravity", "mock-preamble-ask");
    let sess = ctx
        .sessions
        .mutate_session(&key, Some(&sess), |cur| {
            cur.set_session_id("antigravity", "mock-preamble-ask");
            true
        })
        .await
        .unwrap()
        .unwrap();

    let mut rx = ctx.events.subscribe();
    let ask_q = vec![AskQuestionData {
        question: "Pick board".into(),
        options: vec!["STM32H7".into(), "NRF52840".into()],
        is_multi_select: false,
    }];
    let preamble_resp = crate::cli::CliResponse {
        result: "Let me clarify your target board first.".into(),
        session_id: Some("mock-preamble-ask".into()),
        is_error: false,
        stderr: String::new(),
        returncode: Some(0),
    };
    super::turn_stream::drive_stream_events_for_test(
        &ctx,
        &sess,
        "mock-preamble-ask",
        vec![
            crate::cli::StreamEvent::AskQuestion(ask_q),
            crate::cli::StreamEvent::Result(preamble_resp),
        ],
    )
    .await;

    let mut tags = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        match ev {
            WebEvent::AskQuestion { .. } => tags.push("ask".to_string()),
            WebEvent::TurnResult { .. } => tags.push("result".to_string()),
            WebEvent::SessionUpdated { busy, asking, .. } => {
                tags.push(format!("sess:{busy}:{asking}"));
            }
            _ => {}
        }
    }
    assert_eq!(
        tags,
        vec![
            "ask".to_string(),
            "sess:true:true".to_string(),
            "result".to_string(),
            "sess:false:true".to_string(),
        ]
    );
    assert!(ctx.cli.sessions.is_ask_active("mock-preamble-ask").await);
}

#[tokio::test]
async fn test_slash_catalog_and_web_session_delete() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = build_test_ctx(dir.path());
    let app = build_router_with_ctx(ctx.clone());
    let (cookie, csrf) = login_via_magic(&app, &ctx).await;

    // 1. /dashboard/assets/slash.js and i18n.js are served with no-cache
    let asset_res = app
        .clone()
        .oneshot(Request::builder().uri("/dashboard/assets/slash.js").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(asset_res.status(), StatusCode::OK);
    let i18n_res = app
        .clone()
        .oneshot(Request::builder().uri("/dashboard/assets/i18n.js").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(i18n_res.status(), StatusCode::OK);

    // 2. /dashboard/api/models returns provider slash command catalog
    let models_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dashboard/api/models?provider=claude")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(models_res.status(), StatusCode::OK);
    let m_json: serde_json::Value =
        serde_json::from_slice(&to_bytes(models_res.into_body(), 65536).await.unwrap()).unwrap();
    let cmds = m_json["commands"].as_array().unwrap();
    assert!(cmds.iter().any(|c| c["command"] == "simplify" && c["kind"] == "workflow"));
    assert!(cmds.iter().any(|c| c["command"] == "new" && c["kind"] == "builtin"));
    assert!(cmds.iter().any(|c| c["command"] == "compact" && c["kind"] == "workflow"));
    assert!(cmds.iter().any(|c| c["command"] == "stop" && c["kind"] == "builtin"));
    for excluded in &["browser", "schedule", "reset", "abort", "usage"] {
        assert!(!cmds.iter().any(|c| c["command"] == *excluded));
    }

    // 3. Pin, Rename, Prune & Delete sessions (supports both web & tg)
    let pinned_sess = ctx.sessions.create_web_session(Some("Pinned Chat".into()), "antigravity", "gemini-4-hydrogen", None).await.unwrap();
    let pinned_sk = pinned_sess.session_key().storage_key();
    let patch_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri("/dashboard/api/sessions/config")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::json!({
                    "session_key": pinned_sk,
                    "pinned": true,
                    "topic_name": "Starred Spec"
                }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(patch_res.status(), StatusCode::OK);
    let updated_pinned = ctx.sessions.get_active(&pinned_sess.session_key()).await.unwrap().unwrap();
    assert!(updated_pinned.pinned);
    assert_eq!(updated_pinned.topic_name.as_deref(), Some("Starred Spec"));

    let empty_sess = ctx.sessions.create_web_session(Some("Empty Temp".into()), "antigravity", "gemini-4-hydrogen", None).await.unwrap();
    let prune_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/sessions/prune")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::json!({ "transport": "web", "older_than_days": 30 }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(prune_res.status(), StatusCode::OK);
    assert!(ctx.sessions.get_active(&empty_sess.session_key()).await.unwrap().is_none());
    assert!(ctx.sessions.get_active(&pinned_sess.session_key()).await.unwrap().is_some());

    let del_ok = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/dashboard/api/sessions")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::json!({ "session_key": pinned_sk }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(del_ok.status(), StatusCode::OK);
    assert!(ctx.sessions.get_active(&pinned_sess.session_key()).await.unwrap().is_none());

    // 4. HITL status & validation
    let hitl_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dashboard/api/hitl")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hitl_res.status(), StatusCode::OK);
    let h_json: serde_json::Value =
        serde_json::from_slice(&to_bytes(hitl_res.into_body(), 65536).await.unwrap()).unwrap();
    assert!(h_json["sessions"].is_array());

    let bad_hitl = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/hitl/start")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"target":"file:///etc/passwd"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_hitl.status(), StatusCode::BAD_REQUEST);
}

async fn spawn_mock_tg_api() -> (String, Arc<tokio::sync::Mutex<Vec<String>>>) {
    let calls = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let c_clone = calls.clone();
    let router = Router::new().route(
        "/botmocktoken/:method",
        axum::routing::post(move |axum::extract::Path(m): axum::extract::Path<String>, body: String| {
            let log = c_clone.clone();
            async move {
                log.lock().await.push(format!("{m}:{body}"));
                if m == "createForumTopic" {
                    axum::Json(serde_json::json!({
                        "ok": true,
                        "result": { "message_thread_id": 777, "name": "Synced TG Topic" }
                    }))
                } else {
                    axum::Json(serde_json::json!({ "ok": true, "result": true }))
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
    (format!("http://{addr}"), calls)
}

#[tokio::test]
async fn test_telegram_forum_topic_create_rename_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = build_test_ctx(dir.path());
    let (mock_url, calls) = spawn_mock_tg_api().await;
    let bot = teloxide::Bot::new("mocktoken").set_api_url(reqwest::Url::parse(&mock_url).unwrap());
    ctx.set_bot(bot).await;

    let app = build_router_with_ctx(ctx.clone());
    let (cookie, csrf) = login_via_magic(&app, &ctx).await;

    let create_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/sessions")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"transport":"tg","topic_name":"Synced TG Topic","silent_greeting":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_res.status(), StatusCode::CREATED);
    let created: serde_json::Value =
        serde_json::from_slice(&to_bytes(create_res.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(created["session_key"], "tg:12345:777");
    assert_eq!(created["transport"], "tg");

    let rename_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri("/dashboard/api/sessions/config")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"session_key":"tg:12345:777","topic_name":"Renamed TG"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rename_res.status(), StatusCode::OK);

    let del_res = app
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/dashboard/api/sessions")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"session_key":"tg:12345:777","delete_remote_topic":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(del_res.status(), StatusCode::OK);

    let recorded = calls.lock().await.clone();
    assert!(recorded.iter().any(|c| c.starts_with("createForumTopic:")));
    assert!(recorded.iter().any(|c| c.starts_with("editForumTopic:") && c.contains("Renamed TG")));
    assert!(recorded.iter().any(|c| c.starts_with("deleteForumTopic:") && c.contains("777")));
}

#[tokio::test]
async fn test_provider_onboarding_oauth_parser_and_dotenv() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = build_test_ctx(dir.path());
    let app = build_router_with_ctx(ctx.clone());
    let (cookie, csrf) = login_via_magic(&app, &ctx).await;

    // 1. Reconstruct box-wrapped agy Google OAuth URL across 80-col lines
    let raw_pty = "\x1b[32mOpen the URL below:\x1b[0m\r\n ────────────────────\r\n https://accounts.google.com/o/oauth2/auth?client_id=107100\r\n &redirect_uri=https%3A%2F%2Fantigravity.google%2Foauth-callback&state=xyz\r\n ────────────────────\r\n";
    let clean = super::provider_oauth::strip_ansi_codes(raw_pty.as_bytes());
    let url = super::provider_oauth::extract_oauth_url_from_pty(&clean).unwrap();
    assert_eq!(
        url,
        "https://accounts.google.com/o/oauth2/auth?client_id=107100&redirect_uri=https%3A%2F%2Fantigravity.google%2Foauth-callback&state=xyz"
    );

    // 2. Pre-seed agy headless settings.json
    let fake_home = dir.path().join("fake_home");
    super::provider_status::ensure_agy_headless_settings(
        &fake_home.to_string_lossy(),
        &ctx.working_dir,
    );
    let settings_raw = std::fs::read_to_string(fake_home.join(".gemini/antigravity-cli/settings.json")).unwrap();
    assert!(settings_raw.contains("\"onboardingComplete\": true"));

    // 3. POST /dashboard/api/providers/apikey writes 0600 .env & GET /providers/status works
    let key_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/providers/apikey")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"provider":"claude","api_key":"sk-ant-test-onboard"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(key_res.status(), StatusCode::OK);
    let dotenv = super::provider_status::read_dotenv_map(&ctx.tuner_home.join(".env"));
    assert_eq!(dotenv.get("ANTHROPIC_API_KEY").map(String::as_str), Some("sk-ant-test-onboard"));

    let gem_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/dashboard/api/providers/apikey")
                .header(header::COOKIE, &cookie)
                .header("x-tuner-csrf", &csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"provider":"gemini","api_key":"AIzaSyTestGeminiKey1234567890123456789"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(gem_res.status(), StatusCode::OK);
    let dotenv2 = super::provider_status::read_dotenv_map(&ctx.tuner_home.join(".env"));
    assert_eq!(
        dotenv2.get("GEMINI_API_KEY").map(String::as_str),
        Some("AIzaSyTestGeminiKey1234567890123456789")
    );

    let st_res = app
        .oneshot(
            Request::builder()
                .uri("/dashboard/api/providers/status")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(st_res.status(), StatusCode::OK);
    let st_json: serde_json::Value =
        serde_json::from_slice(&to_bytes(st_res.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(st_json["providers"].as_array().unwrap().len(), 3);
    assert_eq!(st_json["gemini_api"]["configured"], true);
    assert_eq!(st_json["gemini_api"]["masked_key"], "AIza…6789");
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_voice_desk_assistant_ws_bridge_tools_barge_in_and_resumption() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as TMsg;

    let _guard = crate::messenger::telegram::TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let en_setup: serde_json::Value =
        serde_json::from_str(&super::voice_protocol::build_live_setup_frame("en", None, None)).unwrap();
    assert_eq!(en_setup["setup"]["generationConfig"]["speechConfig"]["languageCode"], "en-US");
    let auto_setup: serde_json::Value =
        serde_json::from_str(&super::voice_protocol::build_live_setup_frame("auto", None, None)).unwrap();
    assert!(auto_setup["setup"]["generationConfig"]["speechConfig"].get("languageCode").is_none());
    assert!(auto_setup["setup"]["systemInstruction"]["parts"][0]["text"]
        .as_str()
        .unwrap()
        .contains("whichever language"));
    let dir = tempfile::tempdir().unwrap();
    let ctx = build_test_ctx(dir.path());
    super::provider_status::upsert_dotenv_secret(
        &ctx.tuner_home.join(".env"),
        "GEMINI_API_KEY",
        "AIzaSyMockVoiceBridgeTestKey1234567890123",
    )
    .unwrap();

    // 1. Mock upstream Gemini Live server supporting toolCall, audio, interrupted, goAway & sessionResumption!
    let up_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let up_addr = up_listener.local_addr().unwrap();
    let prev_url = std::env::var("TUNER_GEMINI_LIVE_WS_URL").ok();
    unsafe {
        std::env::set_var("TUNER_GEMINI_LIVE_WS_URL", format!("ws://{up_addr}/bidi"));
    }

    let up_task = tokio::spawn(async move {
        // First connection: verify setup -> emit setupComplete + resume handle -> toolCall -> audio + interrupted -> goAway!
        let (tcp1, _) = up_listener.accept().await.unwrap();
        let mut ws1 = tokio_tungstenite::accept_async(tcp1).await.unwrap();
        let setup1: serde_json::Value =
            serde_json::from_str(&ws1.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
        assert_eq!(setup1["setup"]["model"], "models/gemini-3.8-live");
        assert_eq!(setup1["setup"]["generationConfig"]["speechConfig"]["languageCode"], "ko-KR");
        assert_eq!(
            setup1["setup"]["realtimeInputConfig"]["automaticActivityDetection"]["startOfSpeechSensitivity"],
            "START_SENSITIVITY_LOW"
        );
        assert!(setup1["setup"]["contextWindowCompression"]["slidingWindow"].is_object());
        let decls = setup1["setup"]["tools"][0]["functionDeclarations"].as_array().unwrap();
        assert_eq!(decls.len(), 6);

        ws1.send(TMsg::Text(r#"{"setupComplete":{}}"#.into())).await.unwrap();
        ws1.send(TMsg::Text(r#"{"sessionResumptionUpdate":{"resumable":true,"newHandle":"res-handle-777"}}"#.into())).await.unwrap();

        // Wait for user text frame
        let u1: serde_json::Value =
            serde_json::from_str(&ws1.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
        assert_eq!(u1["realtimeInput"]["text"], "새 토픽 열고 확인해줘");

        // Request list_topics tool call
        ws1.send(TMsg::Text(r#"{"toolCall":{"functionCalls":[{"id":"c1","name":"list_topics","args":{}}]}}"#.into())).await.unwrap();
        let tr1: serde_json::Value =
            serde_json::from_str(&ws1.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
        assert_eq!(tr1["toolResponse"]["functionResponses"][0]["name"], "list_topics");

        // Emit PCM24 audio + transcript + barge-in interrupted + goAway!
        ws1.send(TMsg::Text(r#"{"serverContent":{"modelTurn":{"parts":[{"inlineData":{"mimeType":"audio/pcm;rate=24000","data":"AAAA"}}]},"outputTranscription":{"text":"브리핑 완료"},"turnComplete":true}}"#.into())).await.unwrap();
        ws1.send(TMsg::Text(r#"{"serverContent":{"interrupted":true}}"#.into())).await.unwrap();
        ws1.send(TMsg::Text(r#"{"goAway":{"timeLeft":"5s"}}"#.into())).await.unwrap();

        // Second connection: verify seamless sessionResumption reconnect with stored handle!
        let (tcp2, _) = up_listener.accept().await.unwrap();
        let mut ws2 = tokio_tungstenite::accept_async(tcp2).await.unwrap();
        let setup2: serde_json::Value =
            serde_json::from_str(&ws2.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
        assert_eq!(setup2["setup"]["sessionResumption"]["handle"], "res-handle-777");
        ws2.send(TMsg::Text(r#"{"setupComplete":{}}"#.into())).await.unwrap();
        let _ = ws2.next().await;
    });

    // 2. Bind real Axum server and connect client WebSocket to `/dashboard/api/voice/ws`
    let app = build_router_with_ctx(ctx.clone());
    let (cookie, csrf) = login_via_magic(&app, &ctx).await;

    let vst_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dashboard/api/voice/status")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(vst_res.status(), StatusCode::OK);
    let vst_json = body_json(vst_res).await;
    assert_eq!(vst_json["ready"], true);
    assert_eq!(vst_json["csrf_token"], csrf);

    let app_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let app_addr = app_listener.local_addr().unwrap();
    let app_srv = tokio::spawn(async move {
        let _ = axum::serve(app_listener, app).await;
    });

    // Missing CSRF on WebSocket upgrade must be rejected with 403 Forbidden
    let mut bad_req = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
        format!("ws://{app_addr}/dashboard/api/voice/ws?lang=ko"),
    )
    .unwrap();
    bad_req.headers_mut().insert("Cookie", cookie.parse().unwrap());
    bad_req.headers_mut().insert("Origin", format!("http://{app_addr}").parse().unwrap());
    match tokio_tungstenite::connect_async(bad_req).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => {
            assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        }
        other => panic!("Expected HTTP 403 rejection, got {other:?}"),
    }

    let mut req = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
        format!("ws://{app_addr}/dashboard/api/voice/ws?csrf={csrf}&lang=ko"),
    )
    .unwrap();
    req.headers_mut().insert("Cookie", cookie.parse().unwrap());
    req.headers_mut().insert("Origin", format!("http://{app_addr}").parse().unwrap());

    let (mut cws, _) = tokio_tungstenite::connect_async(req).await.unwrap();

    // Receive first `ready`
    let f1: serde_json::Value =
        serde_json::from_str(&cws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
    assert_eq!(f1["type"], "ready");

    // Send text query through voice WS
    cws.send(TMsg::Text(r#"{"type":"text","text":"새 토픽 열고 확인해줘"}"#.into())).await.unwrap();

    let mut saw_tool = false;
    let mut saw_audio = false;
    let mut saw_tx = false;
    let mut saw_interrupted = false;
    let mut saw_resumed_ready = false;
    for _ in 0..10 {
        let Some(Ok(TMsg::Text(raw))) = cws.next().await else { break };
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        match v["type"].as_str().unwrap_or("") {
            "tool_call" if v["name"] == "list_topics" => saw_tool = true,
            "audio" if v["data"] == "AAAA" => saw_audio = true,
            "bot_transcript" if v["text"] == "브리핑 완료" => saw_tx = true,
            "interrupted" => saw_interrupted = true,
            "ready" => {
                saw_resumed_ready = true;
                break;
            }
            _ => {}
        }
    }
    assert!(saw_tool && saw_audio && saw_tx && saw_interrupted && saw_resumed_ready);
    let _ = cws.close(None).await;
    let _ = up_task.await;
    app_srv.abort();
    unsafe {
        match prev_url {
            Some(v) => std::env::set_var("TUNER_GEMINI_LIVE_WS_URL", v),
            None => std::env::remove_var("TUNER_GEMINI_LIVE_WS_URL"),
        }
    }
}
