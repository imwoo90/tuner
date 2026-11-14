//! # Round 11 Zero-Context Audit Regression Tests (`round11_audit_tests.rs`)
//!
//! ## Overview
//! Deterministic regression tests covering Round 11 impeachment fixes:
//! - `ATK-T1-01`: Case-insensitive and canonical model normalization across all 3 providers,
//!   `/status`, session seeding/healing, warm PTY target formatting, and cron/heartbeat configs.
//! - `ATK-T1-02`: 5-tier Codex static effort catalog (`minimal..xhigh`, no `max`/`ultra`).
//! - `ATK-T1-03`: `HeartbeatScheduler` skips unstarted/overdue sessions, runs ephemeral oneshot
//!   processes, and preserves `SessionData.last_active` on suppressed `HEARTBEAT_OK` ticks.
//! - `ATK-T1-04`: `MediaGroupManager` serializes `1500ms` timeout flushes under per-topic
//!   `SessionManager::lock_pool` and flushes buffered albums synchronously before `<1500ms` follow-ups.
//!
//! ## Search Tags
//! #round11-audit, #canonical-model, #codex-efforts, #heartbeat-freshness, #media-group-lock

use std::sync::Arc;
use teloxide::Bot;

use crate::cli::antigravity::{model_spec, AntigravityCli};
use crate::config::{CliConfig, HeartbeatConfig};
use crate::cron::manager::CronManager;
use crate::heartbeat::scheduler::HeartbeatScheduler;
use crate::messenger::telegram::callbacks_effort::group_discovered_models;
use crate::messenger::telegram::media_group::{MediaGroupDeps, MediaGroupManager};
use crate::messenger::telegram::reply::resolve_session_model;
use crate::messenger::telegram::test_helpers::helpers::make_msg;
use crate::messenger::telegram::TopicNameCache;
use crate::session::data::SessionData;
use crate::session::key::SessionKey;
use crate::session::manager::SessionManager;

fn topic_msg_with_id(msg_id: i32, chat_id: i64, topic_id: i64, text: &str) -> teloxide::types::Message {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    make_msg(&format!(
        r#"{{"message_id":{msg_id},"date":1,"chat":{{"id":{chat_id},"type":"supergroup","is_forum":true}},"from":{{"id":100,"is_bot":false,"first_name":"Tester"}},"text":"{escaped}","message_thread_id":{topic_id},"is_topic_message":true}}"#
    ))
}

#[tokio::test]
async fn test_r11_atk01_case_insensitive_and_canonical_models_across_providers() {
    for (raw, expected, expected_pty) in [
        ("GPT-OSS-120B-MEDIUM", "gpt-oss-120b-medium", Some("gpt-oss-120b-medium")),
        ("Gemini-Pro-Orchestrator", "gemini-pro-orchestrator", Some("gemini-pro-orchestrator")),
        ("Gemini-Pro-Orchestrator-Argon", "gemini-pro-orchestrator-argon", Some("gemini-pro-orchestrator-argon")),
        ("Gemini-Pro-Orchestrator-High", "gemini-pro-orchestrator", Some("gemini-pro-orchestrator")),
        ("ANTIGRAVITY-DEFAULT", "antigravity-default", None),
    ] {
        assert_eq!(SessionData::default_model_for_provider("antigravity", Some(raw)), expected);
        assert!(!AntigravityCli::supports_effort(raw));
        let (_, eff) = AntigravityCli::resolve_cli_model_effort(Some(raw), Some("high"));
        assert_eq!(eff, None, "effortless input {raw} must never emit --effort");
        assert_eq!(
            model_spec::format_pty_model_target(raw, Some("high"), Some("high")).as_deref(),
            expected_pty,
            "PTY target for {raw} must never append -high"
        );
    }

    assert_eq!(
        crate::cli::claude::resolve_claude_model_effort(Some("CLAUDE-SONNET-5"), Some("high")),
        (Some("claude-sonnet-5".to_string()), Some("high".to_string()))
    );
    assert_eq!(
        crate::cli::claude::resolve_claude_model_effort(Some("Claude-Haiku-4-5-High"), Some("high")),
        (Some("claude-haiku-4-5".to_string()), None)
    );
    assert_eq!(
        crate::cli::codex::resolve_codex_model_effort(Some("GPT-6.1-SOL"), Some("high")),
        (Some("gpt-6.1-sol".to_string()), Some("high".to_string()))
    );
    assert_eq!(
        crate::cli::codex::resolve_codex_model_effort(Some("GPT-6-ASTRA-ULTRA"), None),
        (Some("gpt-6-astra".to_string()), Some("xhigh".to_string()))
    );

    let temp = tempfile::tempdir().unwrap();
    let mgr = Arc::new(SessionManager::new(
        temp.path().join("sessions.json"),
        60,
        4,
        false,
        "UTC".to_string(),
        None,
    ));
    let key = SessionKey::telegram(500, Some(10));
    let (mut sess, _) = mgr
        .get_or_init_session(&key, "antigravity", "Gemini-4-Argon-Low")
        .await
        .unwrap();
    assert_eq!(sess.model, "gemini-4-argon");
    assert_eq!(sess.effort.as_deref(), Some("low"));
    sess.effort = Some("high".to_string());
    mgr.preserve_session_identity(&sess).await.unwrap();

    let cfg = CliConfig {
        provider: "antigravity".to_string(),
        model: Some("GPT-OSS-120B-MEDIUM".to_string()),
        effort: Some("high".to_string()),
        ..Default::default()
    };
    let msg = topic_msg_with_id(1, 500, 10, "/status");
    assert_eq!(
        resolve_session_model(&msg, &cfg, &mgr).await,
        "gemini-4-argon (effort: high)"
    );
    let unseeded_msg = topic_msg_with_id(2, 500, 99, "/status");
    assert_eq!(
        resolve_session_model(&unseeded_msg, &cfg, &mgr).await,
        "gpt-oss-120b-medium"
    );
}

#[tokio::test]
async fn test_r11_atk02_codex_static_and_live_catalog_are_five_tiers() {
    assert_eq!(
        crate::cli::codex::CODEX_EFFORTS,
        &["minimal", "low", "medium", "high", "xhigh"]
    );
    let discovered = crate::cli::codex::discover_models();
    assert!(!discovered.iter().any(|m| m.ends_with("-max") || m.ends_with("-ultra")));
    for (_, efforts) in group_discovered_models(&discovered) {
        assert_eq!(efforts, vec!["minimal", "low", "medium", "high", "xhigh"]);
    }

    let live_rpc = r#"{"id":2,"result":{"data":[{"id":"gpt-6-astra","model":"gpt-6-astra","hidden":false,"supportedReasoningEfforts":["low","medium","high","xhigh","max","ultra"]},{"id":"gpt-6-luna","model":"gpt-6-luna","hidden":false,"supportedReasoningEfforts":["low","medium","high","xhigh","max"]}]}}"#;
    let parsed_live = crate::cli::common::parse_codex_models_response(live_rpc);
    assert!(!parsed_live.iter().any(|m| m.ends_with("-max") || m.ends_with("-ultra")));
    let grouped_live = group_discovered_models(&parsed_live);
    assert_eq!(grouped_live.len(), 2);
    for (_, efforts) in grouped_live {
        assert_eq!(efforts, vec!["minimal", "low", "medium", "high", "xhigh"]);
    }

    let probed = crate::cli::codex::discover_models_live(&std::collections::HashMap::new()).await;
    assert!(!probed.iter().any(|m| m.ends_with("-max") || m.ends_with("-ultra")));
    for (_, efforts) in group_discovered_models(&probed) {
        if !efforts.is_empty() {
            assert_eq!(efforts, vec!["minimal", "low", "medium", "high", "xhigh"]);
        }
    }
}

#[tokio::test]
async fn test_r11_atk03_heartbeat_skips_unstarted_and_overdue_and_preserves_last_active() {
    let temp = tempfile::tempdir().unwrap();
    let mgr = Arc::new(SessionManager::new(
        temp.path().join("sessions.json"),
        60,
        4,
        false,
        "UTC".to_string(),
        None,
    ));
    let cfg = Arc::new(CliConfig {
        provider: "antigravity".to_string(),
        model: Some("GPT-OSS-120B-MEDIUM".to_string()),
        effort: Some("high".to_string()),
        heartbeat: HeartbeatConfig {
            enabled: true,
            interval_minutes: Some(30),
            quiet_start: None,
            quiet_end: None,
            ack_token: Some("HEARTBEAT_OK".to_string()),
        },
        ..Default::default()
    });
    let cli = Arc::new(AntigravityCli::new((*cfg).clone()));
    let bus = Arc::new(crate::bus::bus::MessageBus::new());
    let hb = HeartbeatScheduler::new(cfg, mgr.clone(), cli, bus);

    let key_unstarted = SessionKey::telegram(601, Some(1));
    let (mut unstarted, _) = mgr.get_or_init_session(&key_unstarted, "antigravity", "gemini-4-argon").await.unwrap();
    let old_ts = (chrono::Utc::now() - chrono::Duration::minutes(45)).to_rfc3339();
    unstarted.last_active = old_ts.clone();

    let key_overdue = SessionKey::telegram(602, Some(2));
    let (mut overdue, _) = mgr.get_or_init_session(&key_overdue, "antigravity", "gemini-4-argon").await.unwrap();
    overdue.set_session_id("antigravity", "overdue-sid");
    let expired_ts = (chrono::Utc::now() - chrono::Duration::minutes(120)).to_rfc3339();
    overdue.last_active = expired_ts.clone();

    let mut all = mgr.load().unwrap();
    all.insert(key_unstarted.storage_key(), unstarted.clone());
    all.insert(key_overdue.storage_key(), overdue.clone());
    mgr.save(&all).unwrap();

    assert!(!hb.has_fresh_active_session(&unstarted));
    assert!(!hb.has_fresh_active_session(&overdue));
    hb.tick().await.unwrap();

    let after_unstarted = mgr.get_active(&key_unstarted).await.unwrap().unwrap();
    let after_overdue = mgr.get_active(&key_overdue).await.unwrap().unwrap();
    assert!(after_unstarted.get_session_id("antigravity").is_empty());
    assert_eq!(after_unstarted.last_active, old_ts);
    assert_eq!(after_overdue.get_session_id("antigravity"), "overdue-sid");
    assert_eq!(after_overdue.last_active, expired_ts);

    let hb_cfg = hb.build_heartbeat_cli_config(&unstarted);
    assert_eq!(hb_cfg.model.as_deref(), Some("gemini-4-argon"));
    let mut mixed = unstarted.clone();
    mixed.model = "Gemini-Pro-Orchestrator-High".to_string();
    let hb_orch = hb.build_heartbeat_cli_config(&mixed);
    assert_eq!(hb_orch.model.as_deref(), Some("gemini-pro-orchestrator"));
    assert_eq!(hb_orch.effort, None);

    let key_fresh = SessionKey::telegram(603, Some(3));
    let (mut fresh, _) = mgr.get_or_init_session(&key_fresh, "antigravity", "gemini-4-argon").await.unwrap();
    fresh.set_session_id("antigravity", "fresh-sid");
    fresh.last_active = old_ts.clone();
    let mut all2 = mgr.load().unwrap();
    all2.insert(key_fresh.storage_key(), fresh.clone());
    mgr.save(&all2).unwrap();

    assert!(hb.has_fresh_active_session(&fresh));
    assert!(!hb.is_cooling_down(&fresh));
    let resp = crate::cli::CliResponse {
        result: "HEARTBEAT_OK".to_string(),
        is_error: false,
        session_id: Some("fresh-sid".to_string()),
        returncode: Some(0),
        stderr: String::new(),
    };
    hb.finish_heartbeat_turn(&fresh, "antigravity", "fresh-sid", &resp).await;
    let post_hb = mgr.get_active(&key_fresh).await.unwrap().unwrap();
    assert_eq!(post_hb.last_active, old_ts, "suppressed HEARTBEAT_OK must not bump last_active");
    assert!(hb.is_cooling_down(&post_hb), "last_heartbeat_at must enforce cooldown");
}

#[tokio::test]
async fn test_r11_atk04_media_group_serializes_under_topic_lock_and_flushes_before_followup() {
    let temp = tempfile::tempdir().unwrap();
    let mgr = Arc::new(SessionManager::new(
        temp.path().join("sessions.json"),
        30,
        4,
        false,
        "UTC".to_string(),
        None,
    ));
    let cfg = Arc::new(CliConfig {
        provider: "antigravity".to_string(),
        model: Some("gemini-3.8-flash".to_string()),
        working_dir: temp.path().to_path_buf(),
        allowed_user_ids: vec![100],
        ..Default::default()
    });
    let cli = Arc::new(AntigravityCli::new((*cfg).clone()));
    let cron = Arc::new(CronManager::new(temp.path().join("cron.json")));
    let topic_cache = Arc::new(TopicNameCache::new());
    let deps = MediaGroupDeps {
        config: cfg.clone(),
        sessions: mgr.clone(),
        cli,
        cron_manager: cron,
        topic_cache,
        bot_username: None,
    };
    let bot = Bot::new("000000:fake_token_for_unit_test");
    let mg = MediaGroupManager::new();

    // 1. Pre-flush before a <1500ms follow-up on the same topic while holding lock_pool
    let key1 = SessionKey::telegram(777, Some(9));
    let (mut s1, _) = mgr.get_or_init_session(&key1, "antigravity", "gemini-3.8-flash").await.unwrap();
    s1.set_session_id("antigravity", "keep-overdue-sid");
    let overdue_ts = (chrono::Utc::now() - chrono::Duration::minutes(90)).to_rfc3339();
    s1.last_active = overdue_ts.clone();
    let mut map = mgr.load().unwrap();
    map.insert(key1.storage_key(), s1);
    mgr.save(&map).unwrap();

    mg.add_message(
        bot.clone(),
        topic_msg_with_id(10, 777, 9, "@model gemini-4-carbon"),
        "grp-fast".to_string(),
        deps.clone(),
    )
    .await;
    {
        let lock = mgr.lock_pool.get((777, Some(9)));
        let _guard = lock.lock().await;
        mg.flush_other_groups_for_topic(&bot, 777, Some(9), None, &deps).await.unwrap();
    }
    let after_fast = mgr.get_active(&key1).await.unwrap().unwrap();
    assert_eq!(after_fast.model, "gemini-4-carbon");
    assert_eq!(after_fast.get_session_id("antigravity"), "keep-overdue-sid");
    assert_eq!(after_fast.last_active, overdue_ts);

    // 2. 1500ms timeout flush blocks while SessionManager::lock_pool is held by an active turn
    let key2 = SessionKey::telegram(778, Some(10));
    let _ = mgr.get_or_init_session(&key2, "antigravity", "gemini-3.8-flash").await.unwrap();
    let topic_lock = mgr.lock_pool.get((778, Some(10)));
    let held_guard = topic_lock.lock().await;

    mg.add_message(
        bot.clone(),
        topic_msg_with_id(20, 778, 10, "@model gemini-pro-orchestrator"),
        "grp-blocked".to_string(),
        deps,
    )
    .await;

    tokio::time::sleep(std::time::Duration::from_millis(1650)).await;
    let while_locked = mgr.get_active(&key2).await.unwrap().unwrap();
    assert_eq!(
        while_locked.model, "gemini-3.8-flash",
        "1500ms media group timer flush must wait on per-topic lock_pool"
    );

    drop(held_guard);
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    let after_unlock = mgr.get_active(&key2).await.unwrap().unwrap();
    assert_eq!(after_unlock.model, "gemini-pro-orchestrator");
}

#[tokio::test]
async fn test_r12_aud001_alias_mixed_case_and_suffixed_models_across_claude_codex_and_status() {
    use crate::messenger::telegram::commands_model::eff_prov_model;
    use crate::messenger::telegram::reply_directives::{clamp_session_effort, session_supports_effort};

    for prov in ["claude", "CLAUDE", "claude-code"] {
        for raw in ["haiku", "HAIKU", "haiku-high", "claude-haiku", "Claude-Haiku-High"] {
            assert!(SessionData::is_model_compatible(raw, prov), "{raw} on {prov}");
            assert_eq!(
                SessionData::canonicalize_model_for_provider(prov, raw).as_deref(),
                Some("claude-haiku-4-5")
            );
            assert!(!session_supports_effort(prov, raw));
            assert_eq!(clamp_session_effort(prov, raw, "high"), None);
        }
        assert_eq!(
            SessionData::canonicalize_model_for_provider(prov, "claude-fable").as_deref(),
            Some("claude-fable-5")
        );
        assert_eq!(
            SessionData::canonicalize_model_for_provider(prov, "claude-sonnet").as_deref(),
            Some("claude-sonnet-5")
        );
        assert_eq!(
            SessionData::canonicalize_model_for_provider(prov, "claude-opus").as_deref(),
            Some("claude-opus-4-8")
        );
    }

    let temp = tempfile::tempdir().unwrap();
    let mgr = Arc::new(SessionManager::new(
        temp.path().join("sessions.json"),
        60,
        4,
        false,
        "UTC".to_string(),
        None,
    ));
    let cfg_claude = CliConfig {
        provider: "CLAUDE".to_string(),
        model: Some("fable-max".to_string()),
        effort: Some("high".to_string()),
        ..Default::default()
    };

    // Persisted raw unhealed `"haiku"` + `"high"` in sessions.json must render `"claude-haiku-4-5"`
    let key_haiku = SessionKey::telegram(810, Some(1));
    let mut raw_sess = SessionData::new(810, "tg".into(), Some(1), "claude-code".into(), "haiku".into());
    raw_sess.effort = Some("high".to_string());
    let mut map = std::collections::HashMap::new();
    map.insert(key_haiku.storage_key(), raw_sess.clone());
    mgr.save(&map).unwrap();

    assert_eq!(
        eff_prov_model(&raw_sess, &cfg_claude),
        ("claude".to_string(), "claude-haiku-4-5".to_string())
    );
    let msg_h = topic_msg_with_id(1, 810, 1, "/status");
    assert_eq!(resolve_session_model(&msg_h, &cfg_claude, &mgr).await, "claude-haiku-4-5");

    let hb_cfg = crate::heartbeat::build_heartbeat_cli_config(&cfg_claude, &raw_sess);
    assert_eq!(hb_cfg.provider, "claude");
    assert_eq!(hb_cfg.model.as_deref(), Some("claude-haiku-4-5"));
    assert_eq!(hb_cfg.effort, None);

    // Healing via get_or_init_session normalizes provider, canonicalizes haiku, and clears effort
    let (healed, _) = mgr.get_or_init_session(&key_haiku, "claude", "claude-fable-5").await.unwrap();
    assert_eq!(healed.provider, "claude");
    assert_eq!(healed.model, "claude-haiku-4-5");
    assert_eq!(healed.effort, None);

    // Suffixed aliases (`fable-max` on claude, `sol-xhigh` on codex) preserve effort across seeding & cron
    let key_fable = SessionKey::telegram(811, Some(2));
    let (seeded_fable, _) = mgr.get_or_init_session(&key_fable, "claude-code", "fable-max").await.unwrap();
    assert_eq!(seeded_fable.provider, "claude");
    assert_eq!(seeded_fable.model, "claude-fable-5");
    assert_eq!(seeded_fable.effort.as_deref(), Some("max"));

    let key_sol = SessionKey::telegram(812, Some(3));
    let (seeded_sol, _) = mgr.get_or_init_session(&key_sol, "OPENAI", "SOL-XHIGH").await.unwrap();
    assert_eq!(seeded_sol.provider, "codex");
    assert_eq!(seeded_sol.model, "gpt-6.1-sol");
    assert_eq!(seeded_sol.effort.as_deref(), Some("xhigh"));

    let job: crate::cron::manager::CronJob = serde_json::from_value(serde_json::json!({
        "id": "j1",
        "title": "j1",
        "schedule": "0 * * * *",
        "task_folder": "j1",
        "agent_instruction": "ping",
        "provider": "OPENAI",
        "model": "sol-xhigh",
        "chat_id": 812,
        "topic_id": 3
    }))
    .unwrap();
    let jc = crate::cron::build_job_cli_config(&CliConfig::default(), temp.path().join("cron.json").as_path(), &job, temp.path().to_path_buf());
    assert_eq!(jc.provider, "codex");
    assert_eq!(jc.model.as_deref(), Some("gpt-6.1-sol"));
    assert_eq!(jc.effort.as_deref(), Some("xhigh"));
}

#[test]
fn test_r12_aud002_antigravity_only_5_5_models_rejected_on_claude() {
    use crate::messenger::telegram::reply_directives::{
        parse_model_effort_args_for_provider, resolve_directive_for_provider, DirectiveOutcome,
    };

    for agy_only in [
        "claude-opus-5-5",
        "claude-sonnet-5-5",
        "opus-5-5",
        "sonnet-5-5",
        "opus5.5",
        "sonnet5.5",
        "claude-opus-5-5-high",
    ] {
        assert_eq!(
            crate::cli::claude::resolve_claude_model_effort(Some(agy_only), None),
            (None, None),
            "{agy_only} must not resolve on claude CLI"
        );
        assert!(!SessionData::is_model_compatible(agy_only, "claude"), "{agy_only} must be incompatible on claude");
        assert_eq!(SessionData::canonicalize_model_for_provider("claude", agy_only), None);
        assert!(
            matches!(
                resolve_directive_for_provider("claude", None, &format!("@model {agy_only} write tests")),
                DirectiveOutcome::IncompatibleModel(_)
            ),
            "@model {agy_only} must be rejected on claude"
        );
        let (parsed_m, _) = parse_model_effort_args_for_provider("claude", agy_only, None);
        assert!(
            !SessionData::is_model_compatible(&parsed_m, "claude"),
            "/model {agy_only} parsed={parsed_m} must be rejected on claude"
        );
        assert!(
            SessionData::is_model_compatible(agy_only, "antigravity"),
            "{agy_only} must remain compatible on antigravity"
        );
    }
}
