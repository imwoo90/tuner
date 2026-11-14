//! # Session Lifecycle Management
//!
//! ## Overview
//! Manages conversation histories, workspace session initializations, and session key resolution
//! for active user and group chat threads.
//!
//! ## Submodules
//! - [`key`] (`key.rs`): Unique session identifiers matching transport, chat, and forum topic mappings.
//! - [`data`] (`data.rs`): In-memory and JSON-serialized structure of active session state.
//! - [`provider_catalog`] (`provider_catalog.rs`): Cross-provider model compatibility, defaults, and `/provider` switching.
//! - [`manager`] (`manager.rs`): Resolves, loads, updates, and resets persisted sessions.
//!
//! ## Search Tags
//! #session-lifecycle, #session-manager, #timezone-reset

use chrono::{DateTime, Duration, LocalResult, TimeZone, Utc};
use chrono_tz::Tz;

pub mod key;
pub mod data;
pub mod provider_catalog;
pub mod manager;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod manager_tests;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub mod topic_history_tests;

/// Evaluate whether the given session has crossed the timezone-based daily reset threshold.
pub fn has_crossed_daily_reset(
    last_active: &DateTime<Utc>,
    now: &DateTime<Utc>,
    user_timezone: &str,
    daily_reset_hour: u32,
) -> bool {
    let tz = user_timezone.parse::<Tz>().unwrap_or(Tz::UTC);
    let now_local = now.with_timezone(&tz);
    let last_local = last_active.with_timezone(&tz);

    let today_reset = match tz.from_local_datetime(
        &now_local.date_naive().and_hms_opt(daily_reset_hour % 24, 0, 0).unwrap_or_default(),
    ) {
        LocalResult::Single(dt) => dt,
        LocalResult::Ambiguous(dt1, _) => dt1,
        LocalResult::None => return true, // Expire on error
    };

    if now_local >= today_reset {
        last_local < today_reset
    } else {
        let yesterday_reset = today_reset - Duration::days(1);
        last_local < yesterday_reset
    }
}

/// Check if the session is fresh according to max messages, idle timeout, and daily reset rules.
pub fn is_session_fresh(
    session: &data::SessionData,
    max_session_messages: Option<i64>,
    idle_timeout_minutes: i64,
    daily_reset_enabled: bool,
    daily_reset_hour: u32,
    user_timezone: &str,
) -> bool {
    is_session_fresh_for_provider(
        session,
        "antigravity",
        max_session_messages,
        idle_timeout_minutes,
        daily_reset_enabled,
        daily_reset_hour,
        user_timezone,
    )
}

pub(crate) fn is_session_fresh_for_provider(
    session: &data::SessionData,
    default_provider: &str,
    max_session_messages: Option<i64>,
    idle_timeout_minutes: i64,
    daily_reset_enabled: bool,
    daily_reset_hour: u32,
    user_timezone: &str,
) -> bool {
    let now = Utc::now();
    let last = match DateTime::parse_from_rfc3339(&session.last_active) {
        Ok(dt) => dt.with_timezone(&Utc),
        Err(_) => return false,
    };

    if let Some(max_msg) = max_session_messages {
        let prov_key = data::SessionData::effective_provider(&session.provider, default_provider);
        if session.get_message_count(prov_key) >= max_msg {
            return false;
        }
    }

    if idle_timeout_minutes > 0
        && now.signed_duration_since(last) >= Duration::minutes(idle_timeout_minutes)
    {
        return false;
    }

    if daily_reset_enabled
        && has_crossed_daily_reset(&last, &now, user_timezone, daily_reset_hour)
    {
        return false;
    }

    true
}

/// Session freshness evaluation re-exports.
pub mod freshness {
    pub use super::{has_crossed_daily_reset, is_session_fresh};
}

pub(crate) fn merge_provider_session(
    cur_data: &mut data::ProviderSessionData,
    incoming: &data::ProviderSessionData,
) {
    let same_session = cur_data.session_id == incoming.session_id;
    if !same_session
        && !cur_data.session_id.is_empty()
        && !cur_data.past_session_ids.contains(&cur_data.session_id)
    {
        cur_data.past_session_ids.push(cur_data.session_id.clone());
    }
    cur_data.session_id = incoming.session_id.clone();
    for past_id in &incoming.past_session_ids {
        if !cur_data.past_session_ids.contains(past_id) {
            cur_data.past_session_ids.push(past_id.clone());
        }
    }
    cur_data.message_count = if same_session {
        cur_data.message_count.max(incoming.message_count)
    } else {
        incoming.message_count
    };
}

pub(crate) fn apply_identity_fields(current: &mut data::SessionData, session: &data::SessionData) {
    current.provider = session.provider.clone();
    current.model = session.model.clone();
    current.language = session.language.clone();
    current.effort = session.effort.clone();
    current.last_progress_msg_id = session.last_progress_msg_id;
    current.pending_attachments = session.pending_attachments.clone();
    current.last_handover_summary = session.last_handover_summary.clone();
    if session.topic_name.is_some() && current.topic_name.is_none() {
        current.topic_name = session.topic_name.clone();
    }
    for (prov, pdata) in &session.provider_sessions {
        let canon = data::SessionData::canonical_provider(prov).unwrap_or(prov.as_str());
        let cur_data = current.provider_sessions.entry(canon.to_string()).or_default();
        merge_provider_session(cur_data, pdata);
    }
    current.last_active = Utc::now().to_rfc3339();
}

pub(crate) fn heal_session_defaults(
    ex: &mut data::SessionData,
    default_prov: &str,
    default_model: &str,
    resolved_name: Option<String>,
) -> bool {
    let mut changed = false;
    let eff_prov = data::SessionData::effective_provider(&ex.provider, default_prov).to_string();
    if ex.provider != eff_prov {
        ex.provider = eff_prov;
        changed = true;
    }
    for canon in ["antigravity", "claude", "codex"] {
        if ex.fold_provider_aliases(canon) {
            changed = true;
        }
    }
    if let Some(canon) = data::SessionData::canonicalize_model_for_provider(&ex.provider, &ex.model) {
        if canon != ex.model {
            if ex.effort.is_none()
                && let Some(se) = data::SessionData::extract_compatible_effort_suffix(&ex.provider, &ex.model)
            {
                ex.effort = Some(se);
            }
            ex.model = canon;
            changed = true;
        }
    } else {
        let p = ex.provider.clone();
        if ex.effort.is_none()
            && let Some(se) = data::SessionData::extract_compatible_effort_suffix(&p, default_model)
        {
            ex.effort = Some(se);
        }
        ex.model = data::SessionData::default_model_for_provider(&p, Some(default_model));
        changed = true;
    }
    let prev_eff = ex.effort.clone();
    let p = ex.provider.clone();
    ex.normalize_effort_for_provider(&p);
    if ex.effort != prev_eff {
        changed = true;
    }
    if resolved_name.is_some() && ex.topic_name != resolved_name {
        ex.topic_name = resolved_name;
        changed = true;
    }
    changed
}

pub(crate) fn reset_existing_topic_session(
    ex: &mut data::SessionData,
    default_prov: &str,
    default_model: &str,
    initial_prov: Option<&str>,
    resolved_name: Option<String>,
) {
    let raw_ap = ex.provider.clone();
    let ap = data::SessionData::effective_provider(&ex.provider, default_prov).to_string();
    ex.clear_provider_session(&ap);
    if !raw_ap.is_empty() && raw_ap != ap {
        ex.clear_provider_session(&raw_ap);
    }
    if let Some(ip) = initial_prov.filter(|p| !p.is_empty()) {
        let cip = data::SessionData::effective_provider(ip, default_prov);
        if cip != ap {
            ex.clear_provider_session(cip);
        }
        if ip != cip && ip != ap {
            ex.clear_provider_session(ip);
        }
    }
    let name_for_heal = if ex.topic_name.is_none() { resolved_name } else { None };
    let _ = heal_session_defaults(ex, default_prov, default_model, name_for_heal);
    ex.last_active = Utc::now().to_rfc3339();
    ex.last_progress_msg_id = None;
}

pub(crate) fn create_seeded_session(
    key: &key::SessionKey,
    provider: &str,
    model: &str,
    resolved_topic_name: Option<String>,
) -> data::SessionData {
    let eff_prov = data::SessionData::effective_provider(provider, "antigravity");
    let suf_eff = data::SessionData::extract_compatible_effort_suffix(eff_prov, model);
    let eff_model = data::SessionData::default_model_for_provider(eff_prov, Some(model));
    let mut s = data::SessionData::new(
        key.chat_id,
        key.transport.clone(),
        key.topic_id,
        eff_prov.to_string(),
        eff_model,
    );
    if let Some(se) = suf_eff {
        s.effort = Some(se);
        s.normalize_effort_for_provider(eff_prov);
    }
    s.topic_name = resolved_topic_name;
    s
}

pub(crate) fn load_sessions_from_path(
    path: &std::path::Path,
) -> Result< std::collections::HashMap<String, data::SessionData>, String> {
    if !path.exists() {
        return Ok(std::collections::HashMap::new());
    }
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    if content.trim().is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let raw_map: std::collections::HashMap<String, serde_json::Value> =
        serde_json::from_str(&content).unwrap_or_default();
    let mut result = std::collections::HashMap::new();
    for (k, v) in raw_map {
        let Ok(pk) = k.parse::<key::SessionKey>() else { continue };
        let Ok(mut sd) = serde_json::from_value::<data::SessionData>(v) else { continue };
        if sd.transport.is_empty() {
            sd.transport = pk.transport.clone();
        }
        result.insert(pk.storage_key(), sd);
    }
    Ok(result)
}

pub(crate) fn save_sessions_to_path(
    path: &std::path::Path,
    sessions: &std::collections::HashMap<String, data::SessionData>,
) -> Result<(), String> {
    let temp_path = path.with_extension("tmp");
    let content = serde_json::to_string_pretty(sessions).map_err(|e| e.to_string())?;
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    std::fs::write(&temp_path, content).map_err(|e| e.to_string())?;
    std::fs::rename(&temp_path, path).map_err(|e| e.to_string())?;
    Ok(())
}
