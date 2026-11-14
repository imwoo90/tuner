//! # Round 9 Zero-Context Audit Regression Tests (`round9_audit_tests.rs`)
//!
//! ## Overview
//! Deterministic unit and mock-process regression tests covering Round 9 invariants:
//! `A1` non-expiring preference/callback/attachment/open-ask routing, `A2` isolated ephemeral
//! `claude`/`codex` oneshot execution without foreground pool eviction or parked asks,
//! `A3` provider-native default model seeding and `"antigravity-default"` effortless handling,
//! and `A4` warm `agy` PTY default-reset protection for live/parked subagents and Native Swarms.
//!
//! ## Search Tags
//! #round9-audit, #session-freshness, #ephemeral-oneshot, #subagent-guard

use std::sync::Arc;
use teloxide::Bot;

use crate::cli::antigravity::AntigravityCli;
use crate::cli::AgentProvider;
use crate::config::CliConfig;
use crate::messenger::telegram::auth::{get_live_open_ask_session, resolve_message_session};
use crate::messenger::telegram::callbacks_effort::handle_standalone_effort_callback;
use crate::messenger::telegram::commands_provider::handle_provider_callback;
use crate::messenger::telegram::media::queue_pending_files;
use crate::messenger::telegram::reply::resolve_session_model;
use crate::messenger::telegram::reply_directives::{clamp_session_effort, session_supports_effort};
use crate::messenger::telegram::test_helpers::helpers::{make_msg, EnvGuard};
use crate::messenger::telegram::TEST_ENV_MUTEX;
use crate::session::data::SessionData;
use crate::session::key::SessionKey;
use crate::session::manager::SessionManager;

fn sample_topic_msg(chat_id: i64, topic_id: i64, text: &str) -> teloxide::types::Message {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    make_msg(&format!(
        r#"{{"message_id":501,"date":1,"chat":{{"id":{chat_id},"type":"supergroup","is_forum":true}},"from":{{"id":100,"is_bot":false,"first_name":"Tester"}},"text":"{escaped}","message_thread_id":{topic_id},"is_topic_message":true}}"#
    ))
}

fn sample_forum_msg(text: &str) -> teloxide::types::Message {
    sample_topic_msg(123, 456, text)
}

#[tokio::test]
async fn test_a1_mutations_preserve_stale_session_and_overdue_last_active() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("sessions.json");
    let mgr = Arc::new(SessionManager::new(
        db,
        30,
        4,
        true,
        "UTC".to_string(),
        Some(5),
    ));
    let key = SessionKey::telegram(123, Some(456));
    let stale_ts = "2020-01-01T00:00:00+00:00";

    let mut seeded = SessionData::new(
        123,
        "tg".to_string(),
        Some(456),
        "antigravity".to_string(),
        "gemini-4-argon".to_string(),
    );
    seeded.set_session_id("antigravity", "sid-keep-alive");
    seeded
        .provider_sessions
        .get_mut("antigravity")
        .unwrap()
        .message_count = 10;
    seeded.last_active = stale_ts.to_string();
    mgr.mutate_session(&key, Some(&seeded), |_| true)
        .await
        .unwrap();

    let (loaded, is_empty) = mgr
        .get_or_init_session(&key, "antigravity", "gemini-3.8-flash")
        .await
        .unwrap();
    assert!(!is_empty);
    assert_eq!(loaded.get_session_id("antigravity"), "sid-keep-alive");
    assert_eq!(loaded.last_active, stale_ts);

    let cfg = CliConfig {
        provider: "antigravity".to_string(),
        working_dir: temp.path().to_path_buf(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(cfg.clone());
    let bot = Bot::new("123:mock");
    let msg = sample_forum_msg("hello");

    queue_pending_files(&msg, &["/tmp/doc.txt".to_string()], &cfg, &mgr).await;
    handle_standalone_effort_callback(&bot, &msg, "high", &mgr, &cfg, &cli).await;
    handle_provider_callback(&bot, &msg, "claude", &mgr, &cfg, &cli).await;
    handle_provider_callback(&bot, &msg, "antigravity", &mgr, &cfg, &cli).await;

    let after = mgr.get_active(&key).await.unwrap().unwrap();
    assert_eq!(after.get_session_id("antigravity"), "sid-keep-alive");
    assert_eq!(after.pending_attachments, vec!["/tmp/doc.txt".to_string()]);
    assert_eq!(after.last_active, stale_ts);

    let (expired, reset_needed) = mgr
        .resolve_session(&key, "antigravity", "gemini-3.8-flash")
        .await
        .unwrap();
    assert!(reset_needed);
    assert!(expired.get_session_id("antigravity").is_empty());
    assert!(expired
        .get_past_session_ids("antigravity")
        .contains(&"sid-keep-alive".to_string()));
}

#[tokio::test]
async fn test_a1_open_ask_and_inflight_turn_bypass_freshness_reset() {
    let temp = tempfile::tempdir().unwrap();
    let mgr = Arc::new(SessionManager::new(
        temp.path().join("sessions.json"),
        1,
        4,
        true,
        "UTC".to_string(),
        Some(1),
    ));
    let key = SessionKey::telegram(123, Some(456));
    let mut seeded = SessionData::new(
        123,
        "tg".to_string(),
        Some(456),
        "antigravity".to_string(),
        "gemini-4-argon".to_string(),
    );
    seeded.set_session_id("antigravity", "mock-ask-999");
    seeded
        .provider_sessions
        .get_mut("antigravity")
        .unwrap()
        .message_count = 50;
    seeded.last_active = "2020-01-01T00:00:00+00:00".to_string();
    mgr.mutate_session(&key, Some(&seeded), |_| true)
        .await
        .unwrap();

    let cfg = CliConfig {
        provider: "antigravity".to_string(),
        working_dir: temp.path().to_path_buf(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(cfg.clone());
    cli.sessions.set_ask_active("mock-ask-999", true).await;

    let msg = sample_forum_msg("Option A");
    let open_ask = get_live_open_ask_session(&msg, &cfg, &mgr, &cli).await;
    assert_eq!(
        open_ask.as_ref().map(|s| s.get_session_id("antigravity")),
        Some("mock-ask-999".to_string())
    );

    cli.sessions.set_ask_active("mock-ask-999", false).await;
    let _turn_guard = cli.begin_topic_turn(123, Some(456));
    let bot = Bot::new("123:mock");
    let resolved = resolve_message_session(&bot, &msg, "follow up", &cfg, &mgr, &cli)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.0.get_session_id("antigravity"), "mock-ask-999");
}

fn write_executable(path: &std::path::Path, body: &str) {
    std::fs::write(path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).unwrap();
    }
}

#[test]
fn test_a2_claude_oneshot_isolated_from_warm_pool_and_denies_ask() {
    let _lock = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let _guard = EnvGuard::new(&temp);
    let script = r#"#!/usr/bin/env python3
import json, sys
for raw in sys.stdin:
    line = raw.strip()
    if not line:
        continue
    msg = json.loads(line)
    mtype = msg.get("type")
    if mtype == "user":
        content = msg.get("message", {}).get("content", "")
        if "TRIGGER_ASK" in content:
            print(json.dumps({
                "type": "control_request",
                "request_id": "ask-1",
                "request": {
                    "subtype": "can_use_tool",
                    "tool_name": "AskUserQuestion",
                    "input": {"questions": [{"question": "Pick?", "options": [{"label": "A"}]}]}
                }
            }), flush=True)
        else:
            print(json.dumps({"type": "result", "session_id": "claude-fg-1", "is_error": False, "result": "FG_OK"}), flush=True)
    elif mtype == "control_response":
        beh = msg.get("response", {}).get("response", {}).get("behavior", "")
        print(json.dumps({"type": "result", "session_id": "claude-cron-1", "is_error": False, "result": f"CRON_{beh}"}), flush=True)
"#;
    write_executable(&temp.path().join("claude"), script);

    let fg_cfg = CliConfig {
        provider: "claude".to_string(),
        model: Some("claude-fable-5".to_string()),
        chat_id: 77701,
        topic_id: Some(11),
        working_dir: temp.path().to_path_buf(),
        ..Default::default()
    };
    let fg_cli = AntigravityCli::new(fg_cfg.clone());
    let cron_cli = AntigravityCli::new_oneshot(fg_cfg);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (fg_resp, fg_pid, cron_resp, post_pid, pending_ask) = rt.block_on(async {
        use futures::StreamExt;
        let mut s = fg_cli
            .send_streaming("hello fg", None, false, temp.path().to_path_buf())
            .await
            .unwrap();
        let mut fg_out = None;
        while let Some(ev) = s.next().await {
            if let crate::cli::StreamEvent::Result(r) = ev {
                fg_out = Some(r);
            }
        }
        let pid_before = crate::cli::claude::active_pid(77701, Some(11)).await;
        let c_out = cron_cli
            .send("TRIGGER_ASK", None, false, temp.path().to_path_buf())
            .await
            .unwrap();
        let pid_after = crate::cli::claude::active_pid(77701, Some(11)).await;
        let ask_flag = crate::cli::claude::has_pending_ask(77701, Some(11), None).await;
        let _ = crate::cli::claude::terminate_topic(77701, Some(11), None).await;
        (fg_out.unwrap(), pid_before, c_out, pid_after, ask_flag)
    });

    assert_eq!(fg_resp.result, "FG_OK");
    assert!(fg_pid.is_some());
    assert_eq!(fg_pid, post_pid);
    assert_eq!(cron_resp.result, "CRON_deny");
    assert!(!pending_ask);
}

#[test]
fn test_a2_codex_oneshot_isolated_from_warm_pool_and_answers_ask() {
    let _lock = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let _guard = EnvGuard::new(&temp);
    let script = r#"#!/usr/bin/env python3
import json, sys, uuid
tid = f"codex-tid-{uuid.uuid4().hex[:8]}"
for raw in sys.stdin:
    line = raw.strip()
    if not line:
        continue
    msg = json.loads(line)
    method = msg.get("method")
    rid = msg.get("id")
    if method == "initialize":
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {"userAgent": "mock"}}), flush=True)
    elif method in ("thread/start", "thread/resume"):
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {"thread": {"id": tid}}}), flush=True)
    elif method == "turn/start":
        txt = msg.get("params", {}).get("input", [{}])[0].get("text", "")
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {"turn": {"id": "t1"}}}), flush=True)
        if "TRIGGER_ASK" in txt:
            print(json.dumps({
                "jsonrpc": "2.0",
                "id": 900,
                "method": "item/tool/requestUserInput",
                "params": {"questions": [{"id": "q1", "question": "Choose?", "options": [{"label": "X"}]}]}
            }), flush=True)
        else:
            print(json.dumps({"jsonrpc": "2.0", "method": "item/completed", "params": {"item": {"type": "agentMessage", "text": "CODEX_FG_OK"}}}), flush=True)
            print(json.dumps({"jsonrpc": "2.0", "method": "turn/completed", "params": {"turn": {"id": "t1"}}}), flush=True)
    elif rid == 900 and "result" in msg:
        print(json.dumps({"jsonrpc": "2.0", "method": "item/completed", "params": {"item": {"type": "agentMessage", "text": "CODEX_CRON_AUTO"}}}), flush=True)
        print(json.dumps({"jsonrpc": "2.0", "method": "turn/completed", "params": {"turn": {"id": "t1"}}}), flush=True)
"#;
    write_executable(&temp.path().join("codex"), script);

    let fg_cfg = CliConfig {
        provider: "codex".to_string(),
        model: Some("gpt-6-astra".to_string()),
        chat_id: 77702,
        topic_id: Some(22),
        working_dir: temp.path().to_path_buf(),
        ..Default::default()
    };
    let fg_cli = AntigravityCli::new(fg_cfg.clone());
    let cron_cli = AntigravityCli::new_oneshot(fg_cfg);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (fg_resp, fg_pid, cron_resp, post_pid, pending_ask) = rt.block_on(async {
        use futures::StreamExt;
        let mut s = fg_cli
            .send_streaming("hello codex", None, false, temp.path().to_path_buf())
            .await
            .unwrap();
        let mut fg_out = None;
        while let Some(ev) = s.next().await {
            if let crate::cli::StreamEvent::Result(r) = ev {
                fg_out = Some(r);
            }
        }
        let pid_before = crate::cli::codex::active_pid(77702, Some(22)).await;
        let c_out = cron_cli
            .send("TRIGGER_ASK", None, false, temp.path().to_path_buf())
            .await
            .unwrap();
        let pid_after = crate::cli::codex::active_pid(77702, Some(22)).await;
        let ask_flag = crate::cli::codex::has_pending_ask(77702, Some(22), None).await;
        let _ = crate::cli::codex::terminate_topic(77702, Some(22), None).await;
        (fg_out.unwrap(), pid_before, c_out, pid_after, ask_flag)
    });

    assert_eq!(fg_resp.result, "CODEX_FG_OK");
    assert!(fg_pid.is_some());
    assert_eq!(fg_pid, post_pid);
    assert_eq!(cron_resp.result, "CODEX_CRON_AUTO");
    assert!(!pending_ask);
}

#[tokio::test]
async fn test_a3_provider_default_models_and_effortless_antigravity_default() {
    assert_eq!(
        SessionData::default_model_for_provider("claude", None),
        "claude-fable-5"
    );
    assert_eq!(
        SessionData::default_model_for_provider("codex", None),
        "gpt-6-astra"
    );
    assert_eq!(
        SessionData::default_model_for_provider("antigravity", None),
        "antigravity-default"
    );
    assert_eq!(
        SessionData::default_model_for_provider("claude", Some("gemini-3.8-flash")),
        "claude-fable-5"
    );
    assert_eq!(
        SessionData::default_model_for_provider("codex", Some("antigravity-default")),
        "gpt-6-astra"
    );

    for prov in ["antigravity", "claude", "codex"] {
        assert!(!session_supports_effort(prov, ""));
        assert!(!session_supports_effort(prov, "antigravity-default"));
        assert_eq!(clamp_session_effort(prov, "", "high"), None);
        assert_eq!(
            clamp_session_effort(prov, "antigravity-default", "high"),
            None
        );
    }

    let temp = tempfile::tempdir().unwrap();
    let mgr = SessionManager::new(
        temp.path().join("sessions.json"),
        30,
        4,
        false,
        "UTC".to_string(),
        None,
    );
    let agy_cfg = CliConfig {
        provider: "antigravity".to_string(),
        model: None,
        effort: Some("high".to_string()),
        ..Default::default()
    };
    let agy_key = SessionKey::telegram(500, Some(1));
    let (mut agy_sess, _) = mgr.resolve_session(&agy_key, "antigravity", "").await.unwrap();
    agy_sess.effort = Some("high".to_string());
    mgr.mutate_session(&agy_key, Some(&agy_sess), |_| true)
        .await
        .unwrap();
    let msg_t1 = sample_topic_msg(500, 1, "/status");
    assert_eq!(
        resolve_session_model(&msg_t1, &agy_cfg, &mgr).await,
        "antigravity-default"
    );

    let claude_cfg = CliConfig {
        provider: "claude".to_string(),
        model: None,
        effort: None,
        ..Default::default()
    };
    let msg_t2 = sample_topic_msg(500, 2, "/status");
    assert_eq!(
        resolve_session_model(&msg_t2, &claude_cfg, &mgr).await,
        "claude-fable-5"
    );
    let codex_cfg = CliConfig {
        provider: "codex".to_string(),
        model: None,
        effort: None,
        ..Default::default()
    };
    let msg_t3 = sample_topic_msg(500, 3, "/status");
    assert_eq!(
        resolve_session_model(&msg_t3, &codex_cfg, &mgr).await,
        "gpt-6-astra"
    );

    let healed = mgr
        .reset_topic_sessions(
            &SessionKey::telegram(500, Some(4)),
            "claude",
            "gemini-3.8-flash",
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(healed.model, "claude-fable-5");
}

#[test]
fn test_a4_warm_pty_default_reset_noop_and_subagent_swarm_deferral() {
    let _lock = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let _guard = EnvGuard::new(&temp);
    let sid = "conv-a4-guard";
    let cfg = CliConfig {
        provider: "antigravity".to_string(),
        working_dir: temp.path().to_path_buf(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(cfg);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (noop_alive, sub_deferred, swarm_deferred, reset_after_kill) = rt.block_on(async {
        let env = cli.build_env();
        cli.sessions
            .ensure_session(sid, temp.path(), "sleep", &["30".to_string()], &env)
            .await
            .unwrap();
        let pid = cli
            .sessions
            .holders
            .lock()
            .await
            .get(sid)
            .and_then(|h| h.child.id())
            .unwrap();

        {
            let l = cli.pty_sync.lock_for(sid).await;
            let _g = l.lock().await;
            cli.sync_locked_idle_pty(sid, 123, Some(456), None).await;
        }
        let still_alive_noop = cli.sessions.is_active(sid).await;

        cli.pty_sync
            .set_applied(sid, pid, Some("gemini-4-argon-high".to_string()))
            .await;
        let gen_dir = temp
            .path()
            .join(format!(".gemini/antigravity-cli/brain/{sid}/.system_generated"));
        let sub_dir = gen_dir.join("subagents");
        std::fs::create_dir_all(&sub_dir).unwrap();
        let sub_file = sub_dir.join("sub1.json");
        std::fs::write(
            &sub_file,
            r#"{"conversationId":"child-sub-1","state":"SUBAGENT_STATE_ALIVE"}"#,
        )
        .unwrap();

        {
            let l = cli.pty_sync.lock_for(sid).await;
            let _g = l.lock().await;
            cli.sync_locked_idle_pty(sid, 123, Some(456), None).await;
        }
        let alive_during_sub = cli.sessions.is_active(sid).await;
        std::fs::remove_file(&sub_file).unwrap();

        let swarm_dir = gen_dir.join("swarm");
        std::fs::create_dir_all(&swarm_dir).unwrap();
        let members_file = swarm_dir.join("members.json");
        std::fs::write(
            &members_file,
            r#"{"members":{"worker1":{"conversation_id":"sw-1","state":"waiting","wake_mode":"mentions"}}}"#,
        )
        .unwrap();

        {
            let l = cli.pty_sync.lock_for(sid).await;
            let _g = l.lock().await;
            cli.sync_locked_idle_pty(sid, 123, Some(456), None).await;
        }
        let alive_during_swarm = cli.sessions.is_active(sid).await;

        std::fs::write(
            &members_file,
            r#"{"members":{"worker1":{"conversation_id":"sw-1","state":"killed","killed":true}}}"#,
        )
        .unwrap();
        {
            let l = cli.pty_sync.lock_for(sid).await;
            let _g = l.lock().await;
            cli.sync_locked_idle_pty(sid, 123, Some(456), None).await;
        }
        let terminated_after = !cli.sessions.is_active(sid).await;
        cli.terminate_session_clean(sid).await;
        (
            still_alive_noop,
            alive_during_sub,
            alive_during_swarm,
            terminated_after,
        )
    });

    assert!(noop_alive, "Default PTY without custom model must not reset");
    assert!(sub_deferred, "Unstarted <20m subagent must defer PTY reset");
    assert!(swarm_deferred, "Parked Native Swarm member must defer PTY reset");
    assert!(reset_after_kill, "Killed swarm must allow clean default PTY reset");
}

#[tokio::test]
async fn test_a1_non_turn_and_rejected_directives_preserve_overdue_session() {
    let temp = tempfile::tempdir().unwrap();
    let mgr = Arc::new(SessionManager::new(
        temp.path().join("sessions.json"),
        30,
        4,
        true,
        "UTC".to_string(),
        Some(5),
    ));
    let key = SessionKey::telegram(123, Some(456));
    let stale_ts = "2020-01-01T00:00:00+00:00";
    let mut s = SessionData::new(123, "tg".into(), Some(456), "claude".into(), "claude-sonnet-4-6".into());
    s.set_session_id("claude", "claude-sid-old");
    s.set_session_id("antigravity", "agy-sid-old");
    s.last_active = stale_ts.into();
    mgr.mutate_session(&key, Some(&s), |_| true).await.unwrap();

    let cfg = CliConfig {
        provider: "claude".into(),
        working_dir: temp.path().to_path_buf(),
        ..Default::default()
    };
    let cli = AntigravityCli::new(cfg.clone());
    let bot = Bot::new("123:mock").set_api_url(reqwest::Url::parse("http://127.0.0.1:1").unwrap());
    let msg = sample_forum_msg("@opus");

    for bad in ["@model gemini-pro-orchestrator Fix bug", "@model claude-opus-4-8 --effort=bogus Fix bug"] {
        assert!(resolve_message_session(&bot, &msg, bad, &cfg, &mgr, &cli).await.unwrap().is_none());
    }
    assert!(resolve_message_session(&bot, &msg, "@opus", &cfg, &mgr, &cli).await.unwrap().is_none());
    queue_pending_files(&msg, &["/tmp/spec.pdf".into()], &cfg, &mgr).await;

    let mid = mgr.get_active(&key).await.unwrap().unwrap();
    assert_eq!(mid.get_session_id("claude"), "claude-sid-old");
    assert_eq!(mid.get_session_id("antigravity"), "agy-sid-old");
    assert_eq!(mid.last_active, stale_ts);
    assert_eq!(mid.model, "claude-opus-4-8");
    assert_eq!(mid.pending_attachments, vec!["/tmp/spec.pdf".to_string()]);

    handle_provider_callback(&bot, &msg, "antigravity", &mgr, &cfg, &cli).await;
    let (turn_sess, rem) = resolve_message_session(&bot, &msg, "@orchestrator Fix bug", &cfg, &mgr, &cli)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rem, "Fix bug");
    assert_eq!(turn_sess.model, "gemini-pro-orchestrator");
    assert_eq!(turn_sess.effort, None);
    assert!(turn_sess.get_session_id("antigravity").is_empty());
    assert!(turn_sess.get_past_session_ids("antigravity").contains(&"agy-sid-old".to_string()));
    assert_eq!(turn_sess.get_session_id("claude"), "claude-sid-old");
}
