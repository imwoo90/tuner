//! # Round 14 Zero-Context Audit Regression Tests (`round14_audit_tests.rs`)
//!
//! ## Overview
//! Deterministic regression tests covering Round 13 PASS-impeachment findings:
//! - `ATK-T1-01`: `MediaGroupManager::spawn_timer_task` exits immediately without CPU spin or
//!   `Arc<MediaGroupDeps>` leak when `<1500ms` pre-flush drops `buf.cancel_tx`.
//! - `ATK-T1-02`: Unhealed alias/mixed-case `SessionData.provider` and `CliConfig.provider`
//!   (`"CLAUDE"`, `"claude-code"`, `"OPENAI"`, `"agy"`) preserve open `ask_question` liveness,
//!   `HeartbeatScheduler::is_session_busy`, `/status`, and `/help` without touching `last_active`.
//! - `ATK-T1-03`: Long-horizon `>=12h` (e.g., `14h`) non-daemon `schedule(DurationSeconds)` timers
//!   block unattended auto-upgrades and warm PTY `/model antigravity-default` resets until expiry.
//!
//! ## Search Tags
//! #round14-audit, #media-group-leak, #provider-alias-liveness, #long-horizon-timer

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use teloxide::Bot;

use crate::bus::bus::MessageBus;
use crate::cli::antigravity::session::AskState;
use crate::cli::antigravity::AntigravityCli;
use crate::cli::{AgentProvider, AskQuestionData, StreamEvent};
use crate::config::CliConfig;
use crate::cron::manager::CronManager;
use crate::heartbeat::scheduler::HeartbeatScheduler;
use crate::messenger::telegram::auth::get_live_open_ask_session;
use crate::messenger::telegram::commands::{build_status_report, resolve_topic_provider};
use crate::messenger::telegram::commands_registry::format_help_text_for_provider;
use crate::messenger::telegram::media_group::{MediaGroupDeps, MediaGroupManager};
use crate::messenger::telegram::test_helpers::helpers::{make_msg, EnvGuard};
use crate::messenger::telegram::TopicNameCache;
use crate::messenger::telegram::TEST_ENV_MUTEX;
use crate::session::data::{ProviderSessionData, SessionData};
use crate::session::key::SessionKey;
use crate::session::manager::SessionManager;
use crate::upgrade::{has_live_or_parked_agy_descendants, inspect_agy_brain_activity};

fn topic_msg(id: i32, chat_id: i64, topic_id: i64, text: &str) -> teloxide::types::Message {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    make_msg(&format!(
        r#"{{"message_id":{id},"date":1,"chat":{{"id":{chat_id},"type":"supergroup","is_forum":true}},"from":{{"id":100,"is_bot":false,"first_name":"Tester"}},"text":"{escaped}","message_thread_id":{topic_id},"is_topic_message":true}}"#
    ))
}

fn write_executable(path: &std::path::Path, content: &str) {
    std::fs::write(path, content).unwrap();
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
}

#[tokio::test]
async fn test_r14_atk01_media_group_preflush_exits_timer_task_without_leak() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = Arc::new(CliConfig {
        provider: "antigravity".to_string(),
        working_dir: temp.path().to_path_buf(),
        ..Default::default()
    });
    let sessions = Arc::new(SessionManager::new(
        temp.path().join("sessions.json"),
        60,
        4,
        false,
        "UTC".to_string(),
        None,
    ));
    let cli = Arc::new(AntigravityCli::new((*cfg).clone()));
    let cron_manager = Arc::new(CronManager::new(temp.path().join("cron_jobs.json")));
    let topic_cache = Arc::new(TopicNameCache::new());
    let deps = MediaGroupDeps {
        config: cfg.clone(),
        sessions: sessions.clone(),
        cli,
        cron_manager,
        topic_cache,
        bot_username: Some("Wimbis_bot".to_string()),
    };

    let baseline_refs = Arc::strong_count(&cfg);
    let mgr = MediaGroupManager::new();
    let bot = Bot::new("123:mock");

    // Queue two uncaptioned messages into `"grp-leak-check"` so both `spawn_timer_task` and
    // the `Some(())` debounce reset branch execute while the 1500ms sleep is still pending.
    mgr.add_message(
        bot.clone(),
        topic_msg(10, 9100, 77, ""),
        "grp-leak-check".to_string(),
        deps.clone(),
    )
    .await;
    assert_eq!(Arc::strong_count(&cfg), baseline_refs + 1);

    mgr.add_message(
        bot.clone(),
        topic_msg(11, 9100, 77, ""),
        "grp-leak-check".to_string(),
        deps.clone(),
    )
    .await;

    // Pre-flush (<1500ms) drops `MediaGroupBuffer` and `buf.cancel_tx`; the timer task MUST
    // break out of `tokio::select!` immediately and drop its `MediaGroupDeps` clone.
    let topic_lock = sessions.lock_pool.get((9100, Some(77)));
    let _guard = topic_lock.lock().await;
    mgr.flush_other_groups_for_topic(&bot, 9100, Some(77), None, &deps)
        .await
        .unwrap();

    for _ in 0..25 {
        if Arc::strong_count(&cfg) == baseline_refs {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        Arc::strong_count(&cfg),
        baseline_refs,
        "spawn_timer_task leaked MediaGroupDeps after <1500ms pre-flush"
    );
}

#[test]
fn test_r14_atk02_unhealed_alias_provider_preserves_open_ask_busy_and_status() {
    let _lock = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let _guard = EnvGuard::new(&temp);

    let script = r#"#!/usr/bin/env python3
import json, sys
print(json.dumps({"type": "system", "subtype": "init", "session_id": "claude-alias-sid"}), flush=True)
for raw in sys.stdin:
    line = raw.strip()
    if not line:
        continue
    msg = json.loads(line)
    if msg.get("type") == "user":
        print(json.dumps({
            "type": "control_request",
            "request_id": "ask-r14",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "AskUserQuestion",
                "input": {"questions": [{"question": "Confirm?", "options": [{"label": "Yes"}]}]}
            }
        }), flush=True)
    elif msg.get("type") == "control_response":
        print(json.dumps({"type": "result", "session_id": "claude-alias-sid", "is_error": False, "result": "DONE"}), flush=True)
"#;
    write_executable(&temp.path().join("claude"), script);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        use futures::StreamExt;
        let chat_id = 9200_i64;
        let topic_id = Some(88_i64);
        let stale_ts = "2020-01-01T00:00:00+00:00";

        let cfg = CliConfig {
            provider: "CLAUDE".to_string(),
            model: Some("fable".to_string()),
            chat_id,
            topic_id,
            working_dir: temp.path().to_path_buf(),
            ..Default::default()
        };
        let cli = AntigravityCli::new(cfg.clone());
        let mut stream = cli
            .send_streaming("ask me", None, false, temp.path().to_path_buf())
            .await
            .unwrap();
        let first_ev = stream.next().await.expect("expected AskQuestion event");
        assert!(matches!(first_ev, StreamEvent::AskQuestion(_)));

        let mgr = Arc::new(SessionManager::new(
            temp.path().join("sessions.json"),
            30,
            4,
            false,
            "UTC".to_string(),
            None,
        ));
        let key = SessionKey::telegram(chat_id, topic_id);
        let mut raw = SessionData::new(chat_id, "tg".into(), topic_id, "claude-code".into(), "fable".into());
        raw.last_active = stale_ts.to_string();
        raw.provider_sessions.insert(
            "CLAUDE".to_string(),
            ProviderSessionData {
                session_id: "claude-alias-sid".to_string(),
                past_session_ids: vec!["claude-old-sid".to_string()],
                message_count: 4,
            },
        );
        let mut map = std::collections::HashMap::new();
        map.insert(key.storage_key(), raw);
        mgr.save(&map).unwrap();

        cli.sessions
            .set_ask_state(
                "claude-alias-sid",
                AskState {
                    msg_id: 77,
                    questions: vec![AskQuestionData {
                        question: "Confirm?".to_string(),
                        options: vec!["Yes".to_string()],
                        is_multi_select: false,
                    }],
                    current_index: 0,
                    answers: vec![String::new()],
                    current_bitmap: String::new(),
                    waiting_for_write_in: false,
                },
            )
            .await;

        let msg = topic_msg(20, chat_id, 88, "Yes");
        assert!(cli.is_ask_backing_alive("claude-code", chat_id, topic_id, "claude-alias-sid").await);
        let open_sess = get_live_open_ask_session(&msg, &cfg, &mgr, &cli).await;
        assert!(open_sess.is_some(), "unhealed alias provider must not wipe live Claude ask");
        assert!(cli.sessions.is_ask_active("claude-alias-sid").await);

        let hb = HeartbeatScheduler::new(
            Arc::new(cfg.clone()),
            mgr.clone(),
            Arc::new(cli.clone()),
            Arc::new(MessageBus::new()),
        );
        assert!(hb.is_session_busy(chat_id, topic_id).await);

        assert_eq!(resolve_topic_provider(&msg, &cfg, &mgr).await, "claude");
        let status = build_status_report(&msg, &cfg, &mgr, &cli).await;
        assert!(status.contains("`claude`"), "status={status}");
        assert!(status.contains("`claude-a` (4 turns"), "status={status}");
        assert!(format_help_text_for_provider("claude-code").contains("Workflow Commands (claude):"));

        // Verify read-only checks did NOT touch `last_active` on disk
        let disk_after = mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(disk_after.last_active, stale_ts);

        crate::cli::claude::submit_ask_answers(chat_id, topic_id, &["Yes".to_string()]).await;
        let _ = stream.next().await;
        let _ = crate::cli::claude::terminate_topic(chat_id, topic_id, None).await;
    });
}

#[test]
fn test_r14_atk03_long_horizon_14h_timer_blocks_upgrade_and_pty_reset() {
    let tmp = tempfile::tempdir().unwrap();
    let brain = tmp.path();
    let t0 = 1_800_000_000_u64;
    let logs = brain.join("long_timer_sid").join(".system_generated").join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let tp = logs.join("transcript.jsonl");
    let lines = [
        r#"{"step_index":1,"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"schedule","args":{"DurationSeconds":"50400","Prompt":"14h_wakeup"}}]}"#,
        r#"{"step_index":2,"type":"GENERIC","status":"RUNNING","content":"Tool is running as a background task with task id: long_timer_sid/task-14h\nTask Description: Timer: 14h"}"#,
        r#"{"step_index":3,"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[]}"#,
    ];
    std::fs::write(&tp, lines.join("\n") + "\n").unwrap();
    filetime::set_file_mtime(&tp, filetime::FileTime::from_unix_time(t0 as i64, 0)).unwrap();

    let roots = vec!["long_timer_sid".to_string()];
    // At T0 + 12h 1m (`43,260s`), transcript mtime > 12h while 14h (`50,400s`) timer is still live!
    let mid_wait = t0 + 43_260;
    assert!(inspect_agy_brain_activity(brain, &roots, mid_wait).0);
    assert!(has_live_or_parked_agy_descendants(
        brain,
        "long_timer_sid",
        mid_wait,
        "HEARTBEAT_OK"
    ));

    // Past 14h + 60s grace (`50,500s`), timer has expired -> idle!
    let post_expiry = t0 + 50_500;
    assert!(!inspect_agy_brain_activity(brain, &roots, post_expiry).0);
    assert!(!has_live_or_parked_agy_descendants(
        brain,
        "long_timer_sid",
        post_expiry,
        "HEARTBEAT_OK"
    ));
}

struct RecordingTransport {
    delivered: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl crate::bus::bus::Transport for RecordingTransport {
    fn transport_name(&self) -> &str {
        "tg"
    }

    async fn deliver(&self, envelope: &crate::bus::envelope::Envelope) -> Result<(), String> {
        self.delivered
            .lock()
            .unwrap()
            .push(envelope.result_text.clone());
        Ok(())
    }

    async fn deliver_broadcast(&self, envelope: &crate::bus::envelope::Envelope) -> Result<(), String> {
        self.deliver(envelope).await
    }
}

#[tokio::test]
async fn test_r15_atk01_heartbeat_ticks_never_exhaust_max_session_messages() {
    let temp = tempfile::tempdir().unwrap();
    let mgr = Arc::new(SessionManager::new(
        temp.path().join("sessions.json"),
        180,
        4,
        false,
        "UTC".to_string(),
        Some(5),
    ));
    let cfg = Arc::new(CliConfig {
        provider: "antigravity".to_string(),
        working_dir: temp.path().to_path_buf(),
        ..Default::default()
    });
    let cli = Arc::new(AntigravityCli::new((*cfg).clone()));
    let bus = Arc::new(MessageBus::new());
    let recorder = Arc::new(RecordingTransport {
        delivered: std::sync::Mutex::new(Vec::new()),
    });
    bus.register_transport(recorder.clone());
    let hb = HeartbeatScheduler::new(cfg, mgr.clone(), cli, bus);

    let key = SessionKey::telegram(9300, Some(99));
    let mut sess = SessionData::new(9300, "tg".into(), Some(99), "antigravity".into(), "gemini-4-argon".into());
    sess.set_session_id("antigravity", "live-max-msg-sid");
    sess.provider_sessions.get_mut("antigravity").unwrap().message_count = 4;
    let mut map = std::collections::HashMap::new();
    map.insert(key.storage_key(), sess.clone());
    mgr.save(&map).unwrap();

    // 3 silent `HEARTBEAT_OK` ticks + 1 genuine non-suppressed alert tick must NEVER bump `message_count`
    for _ in 0..3 {
        let ok_resp = crate::cli::CliResponse {
            result: "  HEARTBEAT_OK \n".to_string(),
            is_error: false,
            session_id: Some("live-max-msg-sid".to_string()),
            returncode: Some(0),
            stderr: String::new(),
        };
        hb.finish_heartbeat_turn(&sess, "antigravity", "live-max-msg-sid", &ok_resp)
            .await;
    }
    let alert_resp = crate::cli::CliResponse {
        result: "Disk 95% full".to_string(),
        is_error: false,
        session_id: Some("live-max-msg-sid".to_string()),
        returncode: Some(0),
        stderr: String::new(),
    };
    hb.finish_heartbeat_turn(&sess, "antigravity", "live-max-msg-sid", &alert_resp)
        .await;

    assert_eq!(recorder.delivered.lock().unwrap().as_slice(), &["Disk 95% full"]);
    let after_hb = mgr.get_active(&key).await.unwrap().unwrap();
    assert_eq!(after_hb.get_message_count("antigravity"), 4);

    // User's 5th real foreground message MUST preserve `live-max-msg-sid` (`reset_needed == false`)
    let (resolved, reset_needed) = mgr
        .resolve_session(&key, "antigravity", "gemini-4-argon")
        .await
        .unwrap();
    assert!(!reset_needed, "background heartbeats must never trigger max_session_messages wipe");
    assert_eq!(resolved.get_session_id("antigravity"), "live-max-msg-sid");
}

#[test]
fn test_r15_atk02_heartbeat_cli_error_and_mismatched_sid_preserve_warm_codex_and_idle_clock() {
    let _lock = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let _guard = EnvGuard::new(&temp);

    // Mock `codex app-server`: succeeds for initial start ("codex-fg-sid"), rejects `thread/resume`
    // (simulating transient lock/failure during oneshot heartbeat), and verifies oneshot never calls `thread/start`.
    let script = r#"#!/usr/bin/env python3
import json, sys
started = 0
for raw in sys.stdin:
    line = raw.strip()
    if not line:
        continue
    msg = json.loads(line)
    mid = msg.get("id")
    method = msg.get("method")
    if method == "initialize":
        print(json.dumps({"jsonrpc": "2.0", "id": mid, "result": {"serverInfo": {"name": "mock"}}}), flush=True)
    elif method == "thread/resume":
        print(json.dumps({"jsonrpc": "2.0", "id": mid, "error": {"code": -32000, "message": "transient lock"}}), flush=True)
    elif method == "thread/start":
        started += 1
        tid = "codex-fg-sid" if started == 1 else "ephemeral-fallback-sid"
        print(json.dumps({"jsonrpc": "2.0", "id": mid, "result": {"thread": {"id": tid}}}), flush=True)
    elif method == "turn/start":
        print(json.dumps({"jsonrpc": "2.0", "id": mid, "result": {"turn": {"id": "t1"}}}), flush=True)
        print(json.dumps({"jsonrpc": "2.0", "method": "item/agentMessage/delta", "params": {"delta": "OK"}}), flush=True)
        print(json.dumps({"jsonrpc": "2.0", "method": "turn/completed", "params": {}}), flush=True)
"#;
    write_executable(&temp.path().join("codex"), script);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let chat_id = 9400_i64;
        let topic_id = Some(44_i64);
        let cfg = CliConfig {
            provider: "codex".to_string(),
            model: Some("gpt-6-astra".to_string()),
            chat_id,
            topic_id,
            working_dir: temp.path().to_path_buf(),
            ..Default::default()
        };
        let fg_cli = AntigravityCli::new(cfg.clone());
        let first = fg_cli.send("hello", None, false, temp.path().to_path_buf()).await.unwrap();
        assert!(!first.is_error);
        assert_eq!(first.session_id.as_deref(), Some("codex-fg-sid"));
        let warm_pid_before = crate::cli::codex::active_pid(chat_id, topic_id).await.expect("warm codex pid");

        // Oneshot resume when `thread/resume` fails MUST error out without falling back to `thread/start`
        let oneshot = AntigravityCli::new_oneshot(cfg.clone());
        let hb_res = oneshot
            .send("heartbeat", Some("codex-fg-sid"), false, temp.path().to_path_buf())
            .await
            .unwrap();
        assert!(hb_res.is_error);
        assert!(hb_res.stderr.contains("Codex oneshot failed to resume thread"));

        let mgr = Arc::new(SessionManager::new(
            temp.path().join("sessions.json"),
            180,
            4,
            false,
            "UTC".to_string(),
            Some(10),
        ));
        let key = SessionKey::telegram(chat_id, topic_id);
        let mut sess = SessionData::new(chat_id, "tg".into(), topic_id, "codex".into(), "gpt-6-astra".into());
        sess.set_session_id("codex", "codex-fg-sid");
        sess.provider_sessions.get_mut("codex").unwrap().message_count = 2;
        let mut map = std::collections::HashMap::new();
        map.insert(key.storage_key(), sess.clone());
        mgr.save(&map).unwrap();

        let bus = Arc::new(MessageBus::new());
        let recorder = Arc::new(RecordingTransport {
            delivered: std::sync::Mutex::new(Vec::new()),
        });
        bus.register_transport(recorder.clone());
        let hb = HeartbeatScheduler::new(Arc::new(cfg.clone()), mgr.clone(), Arc::new(fg_cli.clone()), bus);

        let fixed_epoch = crate::upgrade::activity::now_epoch_secs().saturating_sub(7200);
        crate::upgrade::activity::set_last_activity_epoch_for_test(fixed_epoch);

        // 1) `is_error: true` response; 2) mismatched `session_id`; 3) whitespace-only output
        for bad_resp in [
            hb_res,
            crate::cli::CliResponse {
                result: "Alert from wrong thread".to_string(),
                is_error: false,
                session_id: Some("ephemeral-fallback-sid".to_string()),
                returncode: Some(0),
                stderr: String::new(),
            },
            crate::cli::CliResponse {
                result: "   \n  ".to_string(),
                is_error: false,
                session_id: Some("codex-fg-sid".to_string()),
                returncode: Some(0),
                stderr: String::new(),
            },
        ] {
            hb.finish_heartbeat_turn(&sess, "codex", "codex-fg-sid", &bad_resp)
                .await;
        }

        assert!(recorder.delivered.lock().unwrap().is_empty(), "errors/mismatches/blank must never spam bus");
        assert!(
            crate::upgrade::user_idle_secs() >= 7195,
            "errors/mismatches/blank must not reset LAST_ACTIVITY_EPOCH"
        );
        let disk = mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(disk.get_session_id("codex"), "codex-fg-sid");
        assert!(disk.get_past_session_ids("codex").is_empty());
        assert_eq!(disk.get_message_count("codex"), 2);

        // Next foreground turn MUST reuse the exact same warm Codex PID without SIGKILL
        let second = fg_cli
            .send("second turn", Some("codex-fg-sid"), false, temp.path().to_path_buf())
            .await
            .unwrap();
        assert!(!second.is_error);
        let warm_pid_after = crate::cli::codex::active_pid(chat_id, topic_id).await.expect("warm codex pid");
        assert_eq!(warm_pid_before, warm_pid_after, "foreground warm codex process must survive");

        let _ = crate::cli::codex::terminate_topic(chat_id, topic_id, None).await;
    });
}

#[tokio::test]
async fn test_r15_aud01_resolve_session_empty_provider_uses_configured_provider_message_count() {
    let temp = tempfile::tempdir().unwrap();
    let mgr = SessionManager::new(
        temp.path().join("sessions.json"),
        60,
        4,
        false,
        "UTC".to_string(),
        Some(5),
    );
    let key = SessionKey::telegram(9500, Some(12));
    let mut raw = SessionData::new(9500, "tg".into(), Some(12), String::new(), "claude-fable-5".into());
    raw.provider_sessions.insert(
        "antigravity".to_string(),
        ProviderSessionData {
            session_id: "agy-exhausted".to_string(),
            past_session_ids: Vec::new(),
            message_count: 5,
        },
    );
    raw.provider_sessions.insert(
        "claude".to_string(),
        ProviderSessionData {
            session_id: "claude-live-1".to_string(),
            past_session_ids: Vec::new(),
            message_count: 1,
        },
    );
    let mut map = std::collections::HashMap::new();
    map.insert(key.storage_key(), raw);
    mgr.save(&map).unwrap();

    let (resolved, reset_needed) = mgr
        .resolve_session(&key, "claude", "claude-fable-5")
        .await
        .unwrap();
    assert!(!reset_needed);
    assert_eq!(resolved.provider, "claude");
    assert_eq!(resolved.get_session_id("claude"), "claude-live-1");
}

#[tokio::test]
async fn test_r16_aud01_telegram_handlers_preserve_config_model_effort_suffixes_end_to_end() {
    use crate::messenger::telegram::auth::resolve_message_session;
    use crate::messenger::telegram::lang::handle_lang_command;
    use crate::messenger::telegram::media::queue_pending_files;
    use crate::messenger::telegram::reply::resolve_session_model;
    use crate::messenger::telegram::session_init::configure_session_cli;

    let temp = tempfile::tempdir().unwrap();
    let bot = Bot::new("123:mock").set_api_url(reqwest::Url::parse("http://127.0.0.1:1").unwrap());

    for (tid, prov, cfg_m, exp_prov, exp_m, exp_eff, exp_status) in [
        (201_i64, "antigravity", "gemini-4-hydrogen-xhigh", "antigravity", "gemini-4-hydrogen", Some("xhigh"), "gemini-4-hydrogen (effort: xhigh)"),
        (202_i64, "claude", "fable-max", "claude", "claude-fable-5", Some("max"), "claude-fable-5 (effort: max)"),
        (203_i64, "OPENAI", "  SOL-XHIGH  ", "codex", "gpt-6.1-sol", Some("xhigh"), "gpt-6.1-sol (effort: xhigh)"),
        (204_i64, "antigravity", "GPT-OSS-120B-MEDIUM", "antigravity", "gpt-oss-120b-medium", None, "gpt-oss-120b-medium"),
    ] {
        let mgr = SessionManager::new(
            temp.path().join(format!("sessions_{tid}.json")),
            60,
            4,
            false,
            "UTC".to_string(),
            None,
        );
        let cfg = CliConfig {
            provider: prov.to_string(),
            model: Some(cfg_m.to_string()),
            effort: None,
            working_dir: temp.path().to_path_buf(),
            ..Default::default()
        };
        let cli = AntigravityCli::new(cfg.clone());
        let msg = topic_msg(50, 9600, tid, "run task");

        // Before session creation: `/status` displays suffix effort (and omits on effortless fixed models)
        assert_eq!(resolve_session_model(&msg, &cfg, &mgr).await, exp_status);

        // Attachment queueing + `/lang` + real turn seeding MUST preserve `config.model` suffix effort
        queue_pending_files(&msg, &["/tmp/a.txt".to_string()], &cfg, &mgr).await;
        let _ = handle_lang_command(&bot, &msg, "ko", &cfg, &mgr).await;
        let (sess, _) = resolve_message_session(&bot, &msg, "run task", &cfg, &mgr, &cli)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sess.provider, exp_prov);
        assert_eq!(sess.model, exp_m);
        assert_eq!(sess.effort.as_deref(), exp_eff);

        // CLI config built from seeded `SessionData` + `/status` after session creation MUST match
        let configured = configure_session_cli(&cli, &sess, 9600, Some(tid)).await;
        assert_eq!(configured.config.provider, exp_prov);
        assert_eq!(configured.config.model.as_deref(), Some(exp_m));
        assert_eq!(configured.config.effort.as_deref(), exp_eff);
        assert_eq!(resolve_session_model(&msg, &cfg, &mgr).await, exp_status);
    }

    // Cross-provider healing: `existing.provider == "claude"` with invalid model heals from `config.model = "fable-max"`
    // even when `config.provider == "antigravity"`!
    let cross_mgr = SessionManager::new(
        temp.path().join("sessions_cross.json"),
        60,
        4,
        false,
        "UTC".to_string(),
        None,
    );
    let cross_cfg = CliConfig {
        provider: "antigravity".to_string(),
        model: Some("fable-max".to_string()),
        working_dir: temp.path().to_path_buf(),
        ..Default::default()
    };
    let cross_cli = AntigravityCli::new(cross_cfg.clone());
    let cross_key = SessionKey::telegram(9600, Some(205));
    let mut map = std::collections::HashMap::new();
    map.insert(
        cross_key.storage_key(),
        SessionData::new(9600, "tg".into(), Some(205), "claude".into(), "gemini-4-argon".into()),
    );
    cross_mgr.save(&map).unwrap();
    let cross_msg = topic_msg(51, 9600, 205, "hello");
    let (healed, _) = resolve_message_session(&bot, &cross_msg, "hello", &cross_cfg, &cross_mgr, &cross_cli)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(healed.provider, "claude");
    assert_eq!(healed.model, "claude-fable-5");
    assert_eq!(healed.effort.as_deref(), Some("max"));
}

#[test]
fn test_r16_aud02_and_aud04_ask_send_failure_cancels_pending_ask_and_bogus_effort_hidden() {
    let _lock = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let _guard = EnvGuard::new(&temp);

    let script = r#"#!/usr/bin/env python3
import json, sys
print(json.dumps({"type": "system", "subtype": "init", "session_id": "claude-ask-fail-sid"}), flush=True)
for raw in sys.stdin:
    line = raw.strip()
    if not line:
        continue
    msg = json.loads(line)
    if msg.get("type") == "user":
        print(json.dumps({
            "type": "control_request",
            "request_id": "ask-fail-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "AskUserQuestion",
                "input": {"questions": [{"question": "Proceed?", "options": [{"label": "Yes"}]}]}
            }
        }), flush=True)
"#;
    write_executable(&temp.path().join("claude"), script);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        use futures::StreamExt;
        use crate::messenger::telegram::ask_process::handle_ask_question_event;
        use crate::messenger::telegram::reply::resolve_session_model;

        let chat_id = 9700_i64;
        let topic_id = Some(55_i64);
        let cfg = CliConfig {
            provider: "claude".to_string(),
            model: Some("claude-fable-5".to_string()),
            effort: Some("bogus".to_string()),
            chat_id,
            topic_id,
            working_dir: temp.path().to_path_buf(),
            ..Default::default()
        };
        let mgr = SessionManager::new(
            temp.path().join("sessions.json"),
            60,
            4,
            false,
            "UTC".to_string(),
            None,
        );
        let cli = AntigravityCli::new(cfg.clone());
        let bot = Bot::new("123:mock").set_api_url(reqwest::Url::parse("http://127.0.0.1:1").unwrap());

        // AUD-R15-04: invalid `config.effort = "bogus"` must NEVER render `(effort: bogus)` in `/status`
        let msg = topic_msg(60, chat_id, 55, "/status");
        assert_eq!(resolve_session_model(&msg, &cfg, &mgr).await, "claude-fable-5");

        // AUD-R15-02: if Telegram send fails during `handle_ask_question_event`, `pending_ask` MUST be cancelled
        let mut stream = cli
            .send_streaming("ask", None, false, temp.path().to_path_buf())
            .await
            .unwrap();
        let Some(StreamEvent::AskQuestion(questions)) = stream.next().await else {
            panic!("expected AskQuestion");
        };
        assert!(crate::cli::claude::has_pending_ask(chat_id, topic_id, None).await);

        let key = SessionKey::telegram(chat_id, topic_id);
        let mut sess = SessionData::new(chat_id, "tg".into(), topic_id, "claude".into(), "claude-fable-5".into());
        sess.set_session_id("claude", "claude-ask-fail-sid");
        let mut map = std::collections::HashMap::new();
        map.insert(key.storage_key(), sess.clone());
        mgr.save(&map).unwrap();

        handle_ask_question_event(
            &bot,
            teloxide::types::ChatId(chat_id),
            Some(55),
            questions,
            &sess,
            (&cfg, &cli, &mgr),
        )
        .await
        .unwrap();
        assert!(
            !crate::cli::claude::has_pending_ask(chat_id, topic_id, None).await,
            "failed Telegram ask send must cancel orphaned warm pending_ask"
        );
        let _ = crate::cli::claude::terminate_topic(chat_id, topic_id, None).await;
    });
}

#[test]
fn test_r17_aud01_02_03_multithread_ask_cancel_directives_and_sid_sync() {
    let _lock = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().unwrap();
    let _guard = EnvGuard::new(&temp);
    write_executable(
        &temp.path().join("claude"),
        r#"#!/usr/bin/env python3
import json, sys, time
print(json.dumps({"type":"system","subtype":"init","session_id":"claude-rotated-sid"}), flush=True)
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    obj = json.loads(line)
    if obj.get("type") == "user":
        content = obj.get("message", {}).get("content", "")
        if "ASK_NOW" in str(content):
            print(json.dumps({
                "type":"control_request",
                "session_id":"claude-rotated-sid",
                "request_id":"req-mt-1",
                "request":{
                    "subtype":"can_use_tool",
                    "tool_name":"AskUserQuestion",
                    "input":{"questions":[{"question":"Pick?","options":[{"label":"Yes"},{"label":"No"}]}]}
                }
            }), flush=True)
            time.sleep(0.05)
        else:
            print(json.dumps({"type":"result","session_id":"claude-rotated-sid","result":"FOLLOWUP_OK","is_error":False}), flush=True)
"#,
    );

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        use crate::cli::{AgentProvider, StreamEvent};
        use crate::cron::manager::CronManager;
        use crate::messenger::telegram::ask_process::handle_ask_question_event;
        use crate::messenger::telegram::process_text_with_files;
        use crate::messenger::telegram::TopicNameCache;
        use futures::StreamExt;

        let chat_id = 123i64;
        let topic_id = Some(77i64);
        let key = SessionKey::telegram(chat_id, topic_id);
        let cfg = CliConfig {
            provider: "claude".to_string(),
            chat_id,
            topic_id,
            working_dir: temp.path().to_path_buf(),
            ..Default::default()
        };
        let mgr = Arc::new(SessionManager::new(
            temp.path().join("sessions.json"),
            24,
            100,
            false,
            "UTC".to_string(),
            None,
        ));
        let cli = AntigravityCli::new(cfg.clone());
        let bot = Bot::new("123:mock").set_api_url(reqwest::Url::parse("http://127.0.0.1:1").unwrap());

        // 1) AUD-R16-01 + AUD-R16-03: multi-threaded AskQuestion send failure cancels pending_ask even while producer exits,
        //    and syncs rotated `claude-rotated-sid` into `sessions.json` beforehand.
        let mut sess = SessionData::new(chat_id, "tg".into(), topic_id, "claude".into(), "claude-fable-5".into());
        sess.set_session_id("claude", "stale-pre-turn-sid");
        let mut map = std::collections::HashMap::new();
        map.insert(key.storage_key(), sess.clone());
        mgr.save(&map).unwrap();

        let mut stream = cli.send_streaming("ASK_NOW", Some("stale-pre-turn-sid"), false, temp.path().to_path_buf()).await.unwrap();
        let Some(StreamEvent::AskQuestion(questions)) = stream.next().await else {
            panic!("expected AskQuestion");
        };
        handle_ask_question_event(&bot, teloxide::types::ChatId(chat_id), Some(77), questions, &sess, (&cfg, &cli, &mgr))
            .await
            .unwrap();
        assert!(
            !crate::cli::claude::has_pending_ask(chat_id, topic_id, None).await,
            "multi-threaded ask cancel must never leave orphaned pending_ask"
        );
        let synced_disk = mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(synced_disk.get_session_id("claude"), "claude-rotated-sid");

        // Follow-up normal user turn works cleanly without wedging
        let resp = cli.send("normal turn", Some("claude-rotated-sid"), false, temp.path().to_path_buf()).await.unwrap();
        assert_eq!(resp.result, "FOLLOWUP_OK");
        let _ = crate::cli::claude::terminate_topic(chat_id, topic_id, None).await;

        // 2) AUD-R16-02: standalone `@opus`, `@opus`-captioned upload, and rejected `@model --effort bogus`
        //    preserve open `ask_question` liveness instead of consuming it as write-in text.
        let agy_cfg = CliConfig {
            provider: "antigravity".to_string(),
            chat_id,
            topic_id,
            working_dir: temp.path().to_path_buf(),
            ..Default::default()
        };
        let agy_cli = AntigravityCli::new(agy_cfg.clone());
        let mut agy_sess = SessionData::new(chat_id, "tg".into(), topic_id, "antigravity".into(), "gemini-3.8-flash".into());
        agy_sess.set_session_id("antigravity", "mock-open-ask-17");
        let mut m2 = std::collections::HashMap::new();
        m2.insert(key.storage_key(), agy_sess);
        mgr.save(&m2).unwrap();
        agy_cli.sessions.set_ask_state(
            "mock-open-ask-17",
            crate::cli::antigravity::session::AskState {
                msg_id: 901,
                questions: vec![crate::cli::AskQuestionData {
                    question: "Confirm?".into(),
                    options: vec!["Yes".into(), "No".into()],
                    is_multi_select: false,
                }],
                current_index: 0,
                answers: vec![String::new()],
                current_bitmap: String::new(),
                waiting_for_write_in: false,
            },
        ).await;
        let cron_mgr = CronManager::new(temp.path().join("cron_jobs.json"));
        let topic_cache = TopicNameCache::new();

        let m_pref = topic_msg(70, chat_id, 77, "@carbon");
        let _ = process_text_with_files(&bot, &m_pref, ("@carbon", &["/tmp/a.pdf".to_string()]), &agy_cfg, &mgr, &agy_cli, (&cron_mgr, &topic_cache)).await;
        assert!(agy_cli.sessions.is_ask_active("mock-open-ask-17").await, "@carbon preference must keep open ask alive");
        let after_pref = mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(after_pref.model, "gemini-4-carbon");
        assert_eq!(after_pref.pending_attachments, vec!["/tmp/a.pdf".to_string()]);

        let m_rej = topic_msg(71, chat_id, 77, "@model --effort bogus gemini-4-carbon");
        let _ = process_text_with_files(&bot, &m_rej, ("@model --effort bogus gemini-4-carbon", &[]), &agy_cfg, &mgr, &agy_cli, (&cron_mgr, &topic_cache)).await;
        assert!(agy_cli.sessions.is_ask_active("mock-open-ask-17").await, "rejected @model must keep open ask alive");
    });
}
