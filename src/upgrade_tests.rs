use crate::upgrade::{
    active_background_runs, active_cron_runs, active_foreground_runs, begin_background_run,
    begin_cron_run, begin_foreground_run, evaluate_active_work, has_active_descendants_in_table,
    has_active_descendants_in_table_with_worker, has_any_active_work, inspect_agy_brain_activity,
    inspect_agy_brain_activity_with_ack, is_idle_upgrade_window, is_newer_version,
    now_epoch_secs, observe_activity_epoch, record_user_activity, set_last_activity_epoch_for_test,
    task_tracker::parse_timer_desc_secs, user_idle_secs, ActiveWorkStatus, GithubRelease,
};
use std::path::Path;

#[test]
fn test_version_comparison() {
    assert!(is_newer_version("0.1.0", "0.1.1"));
    assert!(is_newer_version("0.1.0", "v0.2.0"));
    assert!(is_newer_version("1.0.0", "2.0.0"));
    assert!(is_newer_version("0.1.0", "0.1.0.1"));

    assert!(!is_newer_version("0.2.0", "0.1.0"));
    assert!(!is_newer_version("0.1.1", "0.1.1"));
    assert!(!is_newer_version("v1.2.3", "1.2.3"));
    assert!(!is_newer_version("1.0.0", "0.9.9"));
}

#[test]
fn test_hybrid_idle_upgrade_window() {
    assert!(!is_idle_upgrade_window(3, 1799, false));
    assert!(is_idle_upgrade_window(3, 1800, false));
    assert!(is_idle_upgrade_window(4, 2400, false));
    assert!(is_idle_upgrade_window(5, 1800, false));
    assert!(!is_idle_upgrade_window(6, 1800, false));
    assert!(!is_idle_upgrade_window(2, 1800, false));

    assert!(!is_idle_upgrade_window(14, 10799, false));
    assert!(is_idle_upgrade_window(14, 10800, false));
    assert!(is_idle_upgrade_window(22, 15000, false));

    assert!(!is_idle_upgrade_window(4, 36000, true));
    assert!(!is_idle_upgrade_window(15, 36000, true));
}

static ACTIVITY_EPOCH_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn wait_for_global_idle(cli: &crate::cli::antigravity::AntigravityCli) -> bool {
    for _ in 0..900 {
        if !has_any_active_work(cli).await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn test_activity_tracking_and_active_work_guard() {
    let _epoch_guard = ACTIVITY_EPOCH_TEST_LOCK.lock().await;
    let cli = crate::cli::antigravity::AntigravityCli::new(crate::config::CliConfig::default());
    assert!(wait_for_global_idle(&cli).await);

    record_user_activity();
    assert!(user_idle_secs() < 5);

    cli.sessions.set_running("test-session-1", true).await;
    assert!(has_any_active_work(&cli).await);
    cli.sessions.set_running("test-session-1", false).await;
    assert!(wait_for_global_idle(&cli).await);

    {
        let _cron = begin_cron_run();
        assert!(active_cron_runs() >= 1);
        assert!(has_any_active_work(&cli).await);
    }
    assert_eq!(active_cron_runs(), 0);

    for attempt in 0..50 {
        assert!(wait_for_global_idle(&cli).await);
        let aged = now_epoch_secs().saturating_sub(4 * 3600);
        set_last_activity_epoch_for_test(aged);
        observe_activity_epoch(aged);
        let _bg = begin_background_run();
        cli.sessions.set_running("hb-quiet-session", true).await;
        assert_eq!(active_background_runs(), 1);
        let st = evaluate_active_work(&cli).await;
        if active_foreground_runs() > 0 && attempt < 49 {
            cli.sessions.set_running("hb-quiet-session", false).await;
            continue;
        }
        assert_eq!(st, ActiveWorkStatus { is_busy: true, is_user_activity: false });
        assert!(user_idle_secs() >= 4 * 3600 - 5);

        let _fg = begin_foreground_run();
        assert!(active_foreground_runs() >= 1);
        assert_eq!(
            evaluate_active_work(&cli).await,
            ActiveWorkStatus { is_busy: true, is_user_activity: true }
        );
        drop(_fg);
        cli.sessions.set_running("hb-quiet-session", false).await;
        break;
    }
    assert_eq!(active_background_runs(), 0);
    assert!(wait_for_global_idle(&cli).await);
}

#[test]
fn test_quiet_heartbeat_transcript_does_not_reset_idle_or_trigger_120s_window() {
    let tmp = tempfile::tempdir().unwrap();
    let brain = tmp.path();
    let now = 1_794_384_000_u64; // 2026-11-11T08:00:00Z
    let user_ts = now - 4 * 3600; // 2026-11-11T04:00:00Z (4h ago)
    let roots = vec!["hb_parent".to_string()];

    // Quiet `HEARTBEAT_OK` turn finished just 10s ago (`mtime = now - 10`, inside `< 120s`!)
    // MUST NOT mark transcript active and MUST return `user_ts` (4h ago), NOT `now - 10`!
    write_transcript(
        brain,
        "hb_parent",
        &[
            r#"{"step_index":1,"created_at":"2026-11-11T04:00:00Z","type":"USER_INPUT","status":"DONE","content":"real user question"}"#,
            r#"{"step_index":2,"created_at":"2026-11-11T04:00:05Z","type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[],"content":"real answer"}"#,
            r#"{"step_index":3,"created_at":"2026-11-11T07:59:48Z","type":"USER_INPUT","status":"DONE","content":"System self-check: are there any outstanding issues or background alerts to report? Answer 'HEARTBEAT_OK' if everything is running smoothly."}"#,
            r#"{"step_index":4,"created_at":"2026-11-11T07:59:50Z","type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[],"content":"HEARTBEAT_OK\n"}"#,
        ],
        now - 10,
    );
    let (active, eff_mtime) = inspect_agy_brain_activity(brain, &roots, now);
    assert!(!active);
    assert_eq!(eff_mtime, user_ts + 5);
    assert!(is_idle_upgrade_window(14, now.saturating_sub(eff_mtime), active));

    // Custom `ack_token` also suppresses quiet heartbeat, whereas non-suppressed alert marks active!
    write_transcript(
        brain,
        "hb_parent",
        &[
            r#"{"step_index":1,"created_at":"2026-11-11T04:00:00Z","type":"USER_INPUT","status":"DONE","content":"hi"}"#,
            r#"{"step_index":2,"created_at":"2026-11-11T04:00:02Z","type":"PLANNER_RESPONSE","status":"DONE","content":"ok"}"#,
            r#"{"step_index":3,"created_at":"2026-11-11T07:59:48Z","type":"USER_INPUT","status":"DONE","content":"System self-check: are there any outstanding issues or background alerts to report? Answer 'HEARTBEAT_OK' if everything is running smoothly."}"#,
            r#"{"step_index":4,"created_at":"2026-11-11T07:59:50Z","type":"PLANNER_RESPONSE","status":"DONE","content":"CUSTOM_ACK"}"#,
        ],
        now - 10,
    );
    assert!(!inspect_agy_brain_activity_with_ack(brain, &roots, now, "CUSTOM_ACK").0);
    // With default `HEARTBEAT_OK`, `"CUSTOM_ACK"` is a real alert -> active & advances mtime!
    let (alert_active, alert_mtime) = inspect_agy_brain_activity(brain, &roots, now);
    assert!(alert_active);
    assert_eq!(alert_mtime, now - 10);

    // Non-empty `manage_task(Action="list")` prunes finished tasks absent from live running list!
    write_transcript(
        brain,
        "hb_parent",
        &[
            r#"{"step_index":1,"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"run_command","args":{"CommandLine":"python3 -m http.server","IsDaemon":"true"}},{"name":"schedule","args":{"DurationSeconds":"3600"}}]}"#,
            r#"{"step_index":2,"type":"GENERIC","status":"RUNNING","content":"Tool is running as a background task with task id: hb_parent/task-1\nTask Description: server"}"#,
            r#"{"step_index":3,"type":"GENERIC","status":"RUNNING","content":"Tool is running as a background task with task id: hb_parent/task-2\nTask Description: Timer: 60m"}"#,
            r#"{"step_index":4,"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"manage_task","args":{"Action":"list"}}]}"#,
            r#"{"step_index":5,"type":"GENERIC","status":"DONE","content":"You have 1 background task(s) currently running:\n[{\"taskId\":\"hb_parent/task-1\"}]"}"#,
            r#"{"step_index":6,"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[]}"#,
        ],
        now - 300,
    );
    assert!(!inspect_agy_brain_activity(brain, &roots, now).0);
}

#[test]
fn test_proc_descendant_table_filters_zombies_and_mcp() {
    let procs = vec![
        (100, 1, 'S', "agy --prompt-interactive".to_string()),
        (101, 100, 'Z', "[agy-vault] <defunct>".to_string()),
        (
            102,
            100,
            'S',
            "npx -y @playwright/mcp --headless".to_string(),
        ),
        // Child helper inside MCP subtree without "mcp" in its own cmdline MUST be pruned!
        (103, 102, 'S', "/opt/google/chrome/chrome --headless".to_string()),
        // Node-wrapped codex app-server + native vendor binary when idle -> false!
        (
            110,
            1,
            'S',
            "node /home/wimvm/.local/bin/codex app-server --stdio".to_string(),
        ),
        (
            111,
            110,
            'S',
            "/home/wimvm/.local/lib/node_modules/@openai/codex/vendor/codex app-server --stdio"
                .to_string(),
        ),
    ];
    assert!(!has_active_descendants_in_table(&[100, 110], &procs));

    let mut codex_busy = procs.clone();
    codex_busy.push((112, 111, 'R', "bash -lc cargo test".to_string()));
    assert!(has_active_descendants_in_table(&[100, 110], &codex_busy));

    let mut agy_busy = procs.clone();
    agy_busy.push((200, 100, 'S', "python3 gate1_schematic_audit.py".to_string()));
    agy_busy.push((201, 200, 'R', "agy -p --json-schema".to_string()));
    assert!(has_active_descendants_in_table(&[100, 110], &agy_busy));

    // Worker-level daemons (`cloudflared`, `agy-vault`, `agy remote-control serve`) -> idle!
    let mut worker_procs = procs;
    worker_procs.push((50, 1, 'S', "cloudflared tunnel --url http://127.0.0.1:8743".to_string()));
    worker_procs.push((51, 1, 'S', "agy-vault daemon --poll-ms 1000".to_string()));
    worker_procs.push((52, 1, 'S', "agy remote-control serve".to_string()));
    assert!(!has_active_descendants_in_table_with_worker(1, &[], &worker_procs));

    // Direct one-shot CLI child under worker (`agy --output-format json -p ...`) -> active!
    worker_procs.push((53, 1, 'R', "/home/wimvm/.local/bin/agy --output-format json -p check".to_string()));
    assert!(has_active_descendants_in_table_with_worker(1, &[], &worker_procs));
}

#[test]
fn test_cross_turn_batch_isolation_and_compound_timers() {
    assert_eq!(
        parse_timer_desc_secs("Task Description: Timer: 1h 30m 10s, Prompt: check"),
        Some(5410)
    );
    assert_eq!(parse_timer_desc_secs("Task Description: Timer: 2m 30s"), Some(150));

    let tmp = tempfile::tempdir().unwrap();
    let brain = tmp.path();
    let now = 1_800_000_000_u64;
    let roots = vec!["parent".to_string()];

    // Synchronous `"Wait cancelled"` `schedule` (10s, IsDaemon=true) + synchronous `IsDaemon=true`
    // `run_command` MUST NOT poison subsequent 1h30m timer or non-daemon `run_command`!
    write_transcript(
        brain,
        "parent",
        &[
            r#"{"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"schedule","args":{"DurationSeconds":"10","IsDaemon":"true"}},{"name":"run_command","args":{"CommandLine":"echo ok","IsDaemon":"true"}}]}"#,
            r#"{"type":"GENERIC","status":"DONE","content":"Wait cancelled (agent already has undelivered messages)"}"#,
            r#"{"type":"GENERIC","status":"DONE","content":"Command exited with code 0."}"#,
            r#"{"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"schedule","args":{"Prompt":"wakeup"}}]}"#,
            r#"{"type":"GENERIC","status":"RUNNING","content":"Tool is running as a background task with task id: parent/task-77\nTask Description: Timer: 1h 30m, Prompt: wakeup"}"#,
            r#"{"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[]}"#,
        ],
        now - 3600,
    );
    // 1h 30m = 5400s (+60s grace = 5460s) > 3600s elapsed -> MUST still be active!
    assert!(inspect_agy_brain_activity(brain, &roots, now).0);

    // Non-daemon `Cron:` sentinel stays active across > 600 KB transcripts, out-of-order `step_index`,
    // duplicate progressive snapshots, and non-`manage_task` `read_file` outputs containing task strings!
    let padding = "x".repeat(65_000);
    let pad_step = |idx: u64| {
        format!(
            r#"{{"step_index":{},"type":"GENERIC","status":"DONE","content":"{}"}}"#,
            idx, padding
        )
    };
    let mut big_lines = vec![
        // Out-of-order & progressive duplicate `step_index`: step 2 flushed BEFORE step 1!
        r#"{"step_index":2,"type":"GENERIC","status":"RUNNING","content":"Step is still running"}"#.to_string(),
        r#"{"step_index":2,"type":"GENERIC","status":"RUNNING","content":"Tool is running as a background task with task id: parent/task-88\nTask Description: Cron: */10 * * * *, Prompt: sentinel"}"#.to_string(),
        r#"{"step_index":1,"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"schedule","args":{"CronExpression":"*/10 * * * *","IsDaemon":"false"}},{"name":"read_file","args":{"Path":"task_tracker.rs"}}]}"#.to_string(),
        // `read_file` output containing literal task strings MUST NOT wipe `parent/task-88`!
        r#"{"step_index":3,"type":"GENERIC","status":"DONE","content":"No background tasks are currently running.\nTask \"parent/task-88\" cancelled."}"#.to_string(),
    ];
    for idx in 4..15 {
        big_lines.push(pad_step(idx));
    }
    big_lines.push(
        r#"{"step_index":15,"type":"SYSTEM_MESSAGE","status":"DONE","content":"<SYSTEM_MESSAGE>\n[Message] timestamp=2026-11-11T04:00:00Z sender=parent/task-88 priority=HIGH content=tick1\nsender=fake stopped due to server restart"}"#.to_string(),
    );
    big_lines.push(
        r#"{"step_index":16,"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[]}"#.to_string(),
    );
    let refs: Vec<&str> = big_lines.iter().map(|s| s.as_str()).collect();
    write_transcript(brain, "parent", &refs, now - 600);
    assert!(inspect_agy_brain_activity(brain, &roots, now).0);

    // Conversation rewind (`USER_INPUT` repeating an earlier `step_index`) drops abandoned higher steps!
    write_transcript(
        brain,
        "parent",
        &[
            r#"{"step_index":1,"type":"USER_INPUT","status":"DONE","content":"old"}"#,
            r#"{"step_index":2,"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"schedule","args":{"DurationSeconds":"3600"}}]}"#,
            r#"{"step_index":3,"type":"GENERIC","status":"RUNNING","content":"Tool is running as a background task with task id: parent/task-99\nTask Description: Timer: 60m"}"#,
            r#"{"step_index":1,"type":"USER_INPUT","status":"DONE","content":"rewound"}"#,
            r#"{"step_index":2,"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[]}"#,
        ],
        now - 600,
    );
    assert!(!inspect_agy_brain_activity(brain, &roots, now).0);
}

fn write_transcript(brain_root: &Path, cid: &str, lines: &[&str], mtime_epoch: u64) {
    let logs = brain_root.join(cid).join(".system_generated").join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let tp = logs.join("transcript.jsonl");
    std::fs::write(&tp, lines.join("\n") + "\n").unwrap();
    filetime::set_file_mtime(&tp, filetime::FileTime::from_unix_time(mtime_epoch as i64, 0))
        .unwrap();
}

fn register_subagent(brain_root: &Path, parent: &str, child: &str, state: &str) {
    let dir = brain_root
        .join(parent)
        .join(".system_generated")
        .join("subagents");
    std::fs::create_dir_all(&dir).unwrap();
    let meta = serde_json::json!({
        "conversationId": child,
        "state": state,
    });
    std::fs::write(dir.join(format!("{}.json", child)), meta.to_string()).unwrap();
}

#[test]
fn test_agy_brain_recursive_subagent_and_background_task_detection() {
    let tmp = tempfile::tempdir().unwrap();
    let brain = tmp.path();
    let now = 1_800_000_000_u64;

    write_transcript(
        brain,
        "parent",
        &[r#"{"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[]}"#],
        now - 7200,
    );
    register_subagent(brain, "parent", "sub_done", "SUBAGENT_STATE_ALIVE");
    write_transcript(
        brain,
        "sub_done",
        &[r#"{"type":"PLANNER_RESPONSE","status":"DONE"}"#],
        now - 3600,
    );
    register_subagent(brain, "parent", "sub_killed", "SUBAGENT_STATE_KILLED");
    write_transcript(
        brain,
        "sub_killed",
        &[r#"{"type":"GENERIC","status":"DONE"}"#],
        now - 60,
    );

    let roots = vec!["parent".to_string()];
    let (active, max_m) = inspect_agy_brain_activity(brain, &roots, now);
    assert!(!active);
    assert_eq!(max_m, now - 3600);

    // Nested subagent mid-turn while parent is completely idle -> MUST detect active work!
    register_subagent(brain, "sub_done", "nested_live", "SUBAGENT_STATE_ALIVE");
    write_transcript(
        brain,
        "nested_live",
        &[r#"{"type":"GENERIC","status":"DONE","content":"edit_file ok"}"#],
        now - 300,
    );
    let (active2, max_m2) = inspect_agy_brain_activity(brain, &roots, now);
    assert!(active2);
    assert_eq!(max_m2, now - 300);

    // Nested subagent finishes cleanly, and pending 30m timer task while within deadline -> active!
    write_transcript(
        brain,
        "nested_live",
        &[r#"{"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[]}"#],
        now - 1000,
    );
    write_transcript(
        brain,
        "parent",
        &[
            r#"{"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"schedule","args":{"DurationSeconds":"1800","Prompt":"\"check\""}}]}"#,
            r#"{"type":"GENERIC","status":"RUNNING","content":"Tool is running as a background task with task id: parent/task-42\nTask Description: Timer: 30m"}"#,
            r#"{"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[]}"#,
        ],
        now - 900,
    );
    assert!(inspect_agy_brain_activity(brain, &roots, now).0);

    // Timer expires past deadline (created/mtime 2000s ago > 1860s) -> automatically idle!
    write_transcript(
        brain,
        "parent",
        &[
            r#"{"type":"GENERIC","status":"RUNNING","content":"Tool is running as a background task with task id: parent/task-42\nTask Description: Timer: 30m"}"#,
            r#"{"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[]}"#,
        ],
        now - 2000,
    );
    assert!(!inspect_agy_brain_activity(brain, &roots, now).0);

    // Real compact transcript schema: quoted `"args"` `manage_task` kill, `IsDaemon: true`, and `TimerCondition: "any"`
    write_transcript(
        brain,
        "parent",
        &[
            r#"{"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"run_command","args":{"CommandLine":"\"python3 -m http.server\"","IsDaemon":"\"true\""}}]}"#,
            r#"{"type":"GENERIC","status":"RUNNING","content":"Tool is running as a background task with task id: parent/task-50\nTask Description: python3 -m http.server"}"#,
            r#"{"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"schedule","args":{"DurationSeconds":"3600","TimerCondition":"\"any\""}}]}"#,
            r#"{"type":"GENERIC","status":"RUNNING","content":"Tool is running as a background task with task id: parent/task-51\nTask Description: Timer: 60m"}"#,
            r#"{"type":"SYSTEM_MESSAGE","status":"DONE","content":"[Message] sender=subagent-xyz content=done"}"#,
            r#"{"type":"GENERIC","status":"RUNNING","content":"Tool is running as a background task with task id: parent/task-52\nTask Description: Timer: 60m"}"#,
            r#"{"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"manage_task","args":{"Action":"\"kill\"","TaskId":"\"parent/task-52\""}}]}"#,
            r#"{"type":"GENERIC","status":"DONE","content":"Task \"parent/task-52\" cancelled."}"#,
            r#"{"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[]}"#,
        ],
        now - 300,
    );
    assert!(!inspect_agy_brain_activity(brain, &roots, now).0);
}

#[cfg(unix)]
#[test]
fn test_agy_vault_lock_probe_and_zombie_reaping() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_string_lossy().to_string();
    let lock_dir = tmp.path().join(".gemini/agy-vault");
    std::fs::create_dir_all(&lock_dir).unwrap();
    let lock_path = lock_dir.join(".daemon.lock");
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    assert!(!crate::setup::is_agy_vault_lock_held(&home));

    use std::os::unix::io::AsRawFd;
    nix::fcntl::flock(f.as_raw_fd(), nix::fcntl::FlockArg::LockExclusiveNonblock).unwrap();
    assert!(crate::setup::is_agy_vault_lock_held(&home));
    nix::fcntl::flock(f.as_raw_fd(), nix::fcntl::FlockArg::Unlock).unwrap();
    drop(f);
    assert!(!crate::setup::is_agy_vault_lock_held(&home));

    let fake_vault = tmp.path().join("fake-agy-vault");
    std::fs::write(&fake_vault, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&fake_vault, std::fs::Permissions::from_mode(0o755)).unwrap();
    let pid = crate::setup::spawn_reaped_agy_vault(&fake_vault).unwrap();

    let mut reaped = false;
    for _ in 0..40 {
        let stat_path = format!("/proc/{}/stat", pid);
        match std::fs::read_to_string(&stat_path) {
            Err(_) => {
                reaped = true;
                break;
            }
            Ok(s) if !s.contains(") Z ") => {}
            Ok(_) => {}
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    assert!(reaped, "spawned companion PID {} was not reaped cleanly", pid);
}

#[test]
fn test_github_release_deserialization() {
    let json_data = r#"{
        "tag_name": "v0.1.1",
        "body": "Fixed critical issues",
        "assets": [
            {
                "name": "tuner-linux-amd64",
                "browser_download_url": "https://example.com/download/tuner-linux-amd64"
            }
        ]
    }"#;

    let release: Result<GithubRelease, _> = serde_json::from_str(json_data);
    assert!(release.is_ok());
    let release = release.unwrap();
    assert_eq!(release.tag_name, "v0.1.1");
    assert_eq!(release.body.as_deref(), Some("Fixed critical issues"));
    assert_eq!(release.assets.len(), 1);
    assert_eq!(release.assets[0].name, "tuner-linux-amd64");
    assert_eq!(
        release.assets[0].browser_download_url,
        "https://example.com/download/tuner-linux-amd64"
    );
}
