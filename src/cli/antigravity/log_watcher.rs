//! # Antigravity Non-Recursive Log Directory Watchers
//!
//! ## Overview
//! Allocates minimal single-directory (`NonRecursive`) `inotify` / filesystem watchers scoped
//! strictly to active `.system_generated/logs` transcript directories to prevent kernel watch exhaustion.
//!
//! ## Search Tags
//! #inotify-watcher, #non-recursive-watch, #transcript-tailing

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};

pub(crate) fn resolve_watch_target(
    brain_root: &Path,
    session_id: Option<&str>,
    active_path: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(dir) = active_path.and_then(|p| p.parent()).filter(|d| d.is_dir()) {
        return Some(dir.to_path_buf());
    }
    if let Some(sid) = session_id {
        let sdir = brain_root.join(sid);
        let sys = sdir.join(".system_generated");
        let logs = sys.join("logs");
        for cand in [logs, sys, sdir] {
            if cand.is_dir() {
                return Some(cand);
            }
        }
    }
    brain_root.is_dir().then(|| brain_root.to_path_buf())
}

pub(crate) fn sync_nonrecursive_watcher(
    watcher: &mut Option<RecommendedWatcher>,
    watched_dir: &mut Option<PathBuf>,
    target: Option<PathBuf>,
    fs_tx: &tokio::sync::mpsc::UnboundedSender<()>,
) {
    let Some(dir) = target else { return };
    if watched_dir.as_deref() == Some(dir.as_path()) {
        return;
    }
    if watcher.is_none() {
        let tx = fs_tx.clone();
        *watcher = notify::recommended_watcher(move |res: Result<notify::Event, _>| {
            if let Ok(e) = res
                && matches!(e.kind, EventKind::Modify(_) | EventKind::Create(_))
            {
                let _ = tx.send(());
            }
        })
        .ok();
    }
    if let Some(w) = watcher.as_mut() {
        if let Some(prev) = watched_dir.take() {
            let _ = w.unwatch(&prev);
        }
        if w.watch(&dir, RecursiveMode::NonRecursive).is_ok() {
            *watched_dir = Some(dir);
        }
    }
}

/// Creates a single non-recursive filesystem watcher on `<brain_dir>/.system_generated/logs`.
pub(crate) fn create_brain_dir_watcher(
    brain_dir: &Path,
    fs_tx: tokio::sync::mpsc::UnboundedSender<()>,
) -> Option<RecommendedWatcher> {
    let logs = brain_dir.join(".system_generated").join("logs");
    if brain_dir.is_dir() {
        let _ = std::fs::create_dir_all(&logs);
    }
    let tgt = if logs.is_dir() { logs.as_path() } else { brain_dir };
    notify::recommended_watcher(move |res: Result<notify::Event, _>| {
        if let Ok(e) = res
            && matches!(e.kind, EventKind::Modify(_) | EventKind::Create(_))
        {
            let _ = fs_tx.send(());
        }
    })
    .ok()
    .filter(|_| tgt.is_dir())
    .and_then(|mut w| w.watch(tgt, RecursiveMode::NonRecursive).ok().map(|_| w))
}

pub(crate) fn setup_path_watcher(
    path: Option<&PathBuf>,
    tx: tokio::sync::mpsc::UnboundedSender<()>,
) -> Option<RecommendedWatcher> {
    let p = path?.parent()?;
    let mut w = notify::recommended_watcher(move |res: Result<notify::Event, _>| {
        if let Ok(e) = res
            && matches!(e.kind, EventKind::Modify(_) | EventKind::Create(_))
        {
            let _ = tx.send(());
        }
    })
    .ok()?;
    w.watch(p, RecursiveMode::NonRecursive).ok()?;
    Some(w)
}
