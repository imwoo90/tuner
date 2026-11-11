//! # Deep Active-Work & OS Process Tree Inspector
//!
//! ## Overview
//! Detects in-flight foreground turns, background `SessionAsyncObserver` runs, scheduled
//! cron and quiet heartbeat executions, OS `/proc` descendant tool processes (including
//! direct one-shot CLI runs), and delegates Antigravity recursive subagent/task inspection.
//!
//! ## Search Tags
//! #active-work-guard, #proc-tree, #cron-guard, #heartbeat-guard

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static LAST_ACTIVITY_EPOCH: AtomicU64 = AtomicU64::new(0);
static ACTIVE_FG_RUNS: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_CRON_RUNS: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_BG_RUNS: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn now_epoch_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Records the current timestamp as the latest user or agent activity.
pub fn record_user_activity() {
    LAST_ACTIVITY_EPOCH.store(now_epoch_secs(), Ordering::SeqCst);
}

/// Advances the last recorded activity timestamp to at least `epoch` without moving backwards.
pub fn observe_activity_epoch(epoch: u64) {
    let _ = LAST_ACTIVITY_EPOCH.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |curr| {
        (epoch > curr).then_some(epoch)
    });
}

#[cfg(test)]
pub(crate) fn set_last_activity_epoch_for_test(epoch: u64) {
    LAST_ACTIVITY_EPOCH.store(epoch, Ordering::SeqCst);
}

/// Returns the elapsed seconds since the last recorded user or agent activity.
pub fn user_idle_secs() -> u64 {
    let now = now_epoch_secs();
    let last = LAST_ACTIVITY_EPOCH.load(Ordering::SeqCst);
    if last == 0 {
        let _ = LAST_ACTIVITY_EPOCH.compare_exchange(0, now, Ordering::SeqCst, Ordering::SeqCst);
        return 0;
    }
    now.saturating_sub(last)
}

/// Evaluates the hybrid unattended auto-upgrade gate (`03:00..06:00` 30m night OR 3h daytime).
pub fn is_idle_upgrade_window(local_hour: u32, idle_secs: u64, has_active_work: bool) -> bool {
    if has_active_work {
        return false;
    }
    let night_idle = (3..6).contains(&(local_hour % 24)) && idle_secs >= 30 * 60;
    let daytime_long_idle = idle_secs >= 3 * 3600;
    night_idle || daytime_long_idle
}

/// Distinguishes mid-turn busy state from genuine user/agent/cron activity that resets idle timers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveWorkStatus {
    /// `true` when any foreground, background, cron, subagent, task, or child CLI work is running.
    pub is_busy: bool,
    /// `true` when non-quiet user, subagent, task, or cron work is active and should reset idle time.
    pub is_user_activity: bool,
}

/// RAII guard tracking a foreground user CLI stream turn across all providers.
pub struct ForegroundRunGuard(());

impl Drop for ForegroundRunGuard {
    fn drop(&mut self) {
        ACTIVE_FG_RUNS.fetch_sub(1, Ordering::SeqCst);
        record_user_activity();
    }
}

/// Marks the start of a foreground user turn and refreshes user activity on start and drop.
pub fn begin_foreground_run() -> ForegroundRunGuard {
    ACTIVE_FG_RUNS.fetch_add(1, Ordering::SeqCst);
    record_user_activity();
    ForegroundRunGuard(())
}

/// Returns the number of foreground user CLI turns currently executing.
pub fn active_foreground_runs() -> usize {
    ACTIVE_FG_RUNS.load(Ordering::SeqCst)
}

/// RAII guard tracking an active scheduled or manual cron job run.
pub struct CronRunGuard(());

impl Drop for CronRunGuard {
    fn drop(&mut self) {
        ACTIVE_CRON_RUNS.fetch_sub(1, Ordering::SeqCst);
        record_user_activity();
    }
}

/// Marks the start of a cron execution and returns an RAII guard that clears it on drop.
pub fn begin_cron_run() -> CronRunGuard {
    ACTIVE_CRON_RUNS.fetch_add(1, Ordering::SeqCst);
    record_user_activity();
    CronRunGuard(())
}

/// Returns the number of cron jobs currently executing.
pub fn active_cron_runs() -> usize {
    ACTIVE_CRON_RUNS.load(Ordering::SeqCst)
}

/// RAII guard tracking an internal quiet background turn (such as `HeartbeatScheduler`)
/// without resetting user inactivity timestamps.
pub struct BackgroundRunGuard(());

impl Drop for BackgroundRunGuard {
    fn drop(&mut self) {
        ACTIVE_BG_RUNS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Marks the start of a background system turn and returns an RAII guard that clears it on drop.
pub fn begin_background_run() -> BackgroundRunGuard {
    ACTIVE_BG_RUNS.fetch_add(1, Ordering::SeqCst);
    BackgroundRunGuard(())
}

/// Returns the number of quiet background system turns currently executing.
pub fn active_background_runs() -> usize {
    ACTIVE_BG_RUNS.load(Ordering::SeqCst)
}

fn is_persistent_mcp_daemon(cmdline: &str) -> bool {
    let lower = cmdline.to_ascii_lowercase();
    lower.contains("@modelcontextprotocol")
        || lower.contains("mcp-server")
        || lower.contains("-mcp")
        || lower.contains("/mcp")
}

fn has_warm_cli_token(cmdline: &str) -> bool {
    cmdline.split_whitespace().any(|tok| {
        let trimmed = tok.trim_matches(|c: char| c == '"' || c == '\'');
        let bin = trimmed.rsplit('/').next().unwrap_or(trimmed);
        matches!(bin, "agy" | "claude" | "codex")
    })
}

fn is_warm_cli_infrastructure(cmdline: &str) -> bool {
    has_warm_cli_token(cmdline)
        && ((cmdline.contains("app-server") && cmdline.contains("--stdio"))
            || cmdline.contains("--prompt-interactive")
            || (cmdline.contains("--input-format") && cmdline.contains("stream-json")))
}

fn is_direct_oneshot_cli_child(cmdline: &str) -> bool {
    has_warm_cli_token(cmdline)
        && !is_warm_cli_infrastructure(cmdline)
        && !cmdline.contains("remote-control")
        && !cmdline.contains("mic-serve")
}

/// Evaluates a `(pid, ppid, state, cmdline)` table with worker PID awareness (`self_pid`),
/// catching both descendants of warm CLI roots and direct one-shot CLI children of `self_pid`.
pub fn has_active_descendants_in_table_with_worker(
    self_pid: u32,
    root_pids: &[u32],
    procs: &[(u32, u32, char, String)],
) -> bool {
    let mut roots: Vec<u32> = root_pids.to_vec();
    let mut children: HashMap<u32, Vec<(u32, char, &str)>> = HashMap::new();
    for (pid, ppid, state, cmd) in procs {
        if self_pid != 0 && *ppid == self_pid && *state != 'Z' && *state != 'X' {
            if is_warm_cli_infrastructure(cmd) {
                roots.push(*pid);
            } else if is_direct_oneshot_cli_child(cmd) {
                return true;
            }
        }
        children.entry(*ppid).or_default().push((*pid, *state, cmd.as_str()));
    }
    if roots.is_empty() {
        return false;
    }
    let mut visited: HashSet<u32> = roots.iter().copied().collect();
    let mut stack = roots;
    while let Some(curr) = stack.pop() {
        let Some(kids) = children.get(&curr) else {
            continue;
        };
        for &(cpid, state, cmd) in kids {
            if !visited.insert(cpid) || state == 'Z' || state == 'X' {
                continue;
            }
            if is_persistent_mcp_daemon(cmd) {
                continue;
            }
            if is_warm_cli_infrastructure(cmd) {
                stack.push(cpid);
                continue;
            }
            return true;
        }
    }
    false
}

/// Evaluates a `(pid, ppid, state, cmdline)` process table for `root_pids`.
pub fn has_active_descendants_in_table(
    root_pids: &[u32],
    procs: &[(u32, u32, char, String)],
) -> bool {
    has_active_descendants_in_table_with_worker(0, root_pids, procs)
}

/// Scans `/proc` and returns `true` if any warm CLI process or direct worker child CLI command is active.
pub fn has_active_descendant_processes(root_pids: &[u32]) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    let mut procs = Vec::new();
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some(rparen) = stat.rfind(')') else {
            continue;
        };
        let mut fields = stat[rparen + 1..].split_whitespace();
        let state = fields.next().and_then(|s| s.chars().next()).unwrap_or('S');
        let Some(ppid) = fields.next().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let cmdline = std::fs::read(entry.path().join("cmdline"))
            .map(|b| {
                b.into_iter()
                    .map(|c| if c == 0 { b' ' } else { c })
                    .collect::<Vec<u8>>()
            })
            .map(|b| String::from_utf8_lossy(&b).trim().to_string())
            .unwrap_or_default();
        procs.push((pid, ppid, state, cmdline));
    }
    has_active_descendants_in_table_with_worker(std::process::id(), root_pids, &procs)
}

async fn collect_live_agy_holders(
    cli: &crate::cli::antigravity::AntigravityCli,
) -> (Vec<String>, Vec<u32>) {
    let mut holders = cli.sessions.holders.lock().await;
    let mut sids = Vec::new();
    let mut pids = Vec::new();
    for (sid, h) in holders.iter_mut() {
        if h.child.try_wait().map(|s| s.is_none()).unwrap_or(false) {
            sids.push(sid.clone());
            if let Some(pid) = h.child.id() {
                pids.push(pid);
            }
        }
    }
    (sids, pids)
}

async fn inspect_live_agy_brain(
    cli: &crate::cli::antigravity::AntigravityCli,
    live_agy_sids: &[String],
) -> bool {
    let brain_root =
        crate::cli::antigravity::events::agy_state_root(Some(&cli.build_env())).join("brain");
    let ack = cli.config.heartbeat.ack_token.as_deref().unwrap_or("HEARTBEAT_OK");
    let (brain_active, max_mtime) = super::brain_inspector::inspect_agy_brain_activity_with_ack(
        &brain_root,
        live_agy_sids,
        now_epoch_secs(),
        ack,
    );
    if max_mtime > 0 {
        observe_activity_epoch(max_mtime);
    }
    brain_active
}

/// Evaluates both mid-turn busy state and whether non-quiet user/subagent/cron work is active.
pub async fn evaluate_active_work(
    cli: &crate::cli::antigravity::AntigravityCli,
) -> ActiveWorkStatus {
    let running_len = cli.sessions.running_runs.lock().await.len();
    if active_foreground_runs() > 0
        || active_cron_runs() > 0
        || !cli.sessions.async_runs.lock().await.is_empty()
        || !cli.sessions.active_asks.lock().await.is_empty()
        || !cli.sessions.boot_cancels.lock().await.is_empty()
    {
        return ActiveWorkStatus { is_busy: true, is_user_activity: true };
    }
    let (live_agy_sids, warm_pids) = collect_live_agy_holders(cli).await;
    let bg_runs = active_background_runs();
    if bg_runs > 0 {
        let brain_active = inspect_live_agy_brain(cli, &live_agy_sids).await;
        return ActiveWorkStatus {
            is_busy: true,
            is_user_activity: running_len > bg_runs || brain_active,
        };
    }
    if running_len > 0
        || crate::cli::claude::is_running(0, None, None).await
        || crate::cli::codex::is_running(0, None, None).await
        || has_active_descendant_processes(&warm_pids)
    {
        return ActiveWorkStatus { is_busy: true, is_user_activity: true };
    }
    let brain_active = inspect_live_agy_brain(cli, &live_agy_sids).await;
    ActiveWorkStatus { is_busy: brain_active, is_user_activity: brain_active }
}

pub(crate) async fn detect_any_active_work(
    cli: &crate::cli::antigravity::AntigravityCli,
) -> bool {
    evaluate_active_work(cli).await.is_busy
}
