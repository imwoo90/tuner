//! # Antigravity Model & Reasoning Effort Specification (`model_spec.rs`)
//!
//! ## Overview
//! Centralizes model alias canonicalization, TTL-refreshed live `agy models` capability
//! caching, fixed single-variant & effortless model detection (`gemini-pro-orchestrator`,
//! `gemini-pro-orchestrator-argon`, `gpt-oss-120b-medium`, `antigravity-default`), warm PTY
//! `/model` target formatting, and per-model reasoning `--effort` clamping.
//!
//! ## Search Tags
//! #model-spec, #orchestrator-model, #effort-clamping, #agy-catalog

use crate::cli::common::strip_effort_suffix;
use std::sync::{LazyLock, RwLock};
use std::time::Instant;

type CatalogState = (Option<Instant>, Vec<String>);

static LIVE_AGY_CATALOG: LazyLock<RwLock<CatalogState>> =
    LazyLock::new(|| RwLock::new((None, Vec::new())));

#[cfg(not(test))]
static CATALOG_REFRESH_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

#[cfg(not(test))]
const CATALOG_TTL: std::time::Duration = std::time::Duration::from_secs(600);

/// Updates the in-memory `agy models` catalog used to detect newly introduced
/// effortless, single-variant, or extended-effort models dynamically at runtime.
/// No-op under `#[cfg(test)]` to keep parallel unit tests isolated from host `agy`.
pub fn update_live_catalog(models: &[String]) {
    #[cfg(test)]
    {
        let _ = models;
    }
    #[cfg(not(test))]
    {
        if models.is_empty() {
            return;
        }
        if let Ok(mut guard) = LIVE_AGY_CATALOG.write() {
            *guard = (Some(Instant::now()), models.to_vec());
        }
    }
}

#[cfg(not(test))]
fn is_catalog_fresh() -> bool {
    LIVE_AGY_CATALOG
        .read()
        .ok()
        .is_some_and(|g| !g.1.is_empty() && g.0.is_some_and(|t| t.elapsed() < CATALOG_TTL))
}

fn with_live_catalog<R>(f: impl FnOnce(Option<&[String]>) -> R) -> R {
    let guard = LIVE_AGY_CATALOG.read().ok();
    let cat = guard.as_ref().map(|g| g.1.as_slice()).filter(|c| !c.is_empty());
    f(cat)
}

/// Lazily populates or refreshes the `agy models` capability cache when empty or older
/// than 10 minutes (`CATALOG_TTL`). No-op under `#[cfg(test)]`.
pub async fn ensure_live_catalog_fresh() {
    #[cfg(not(test))]
    {
        if is_catalog_fresh() {
            return;
        }
        let _guard = CATALOG_REFRESH_LOCK.lock().await;
        if !is_catalog_fresh() {
            let models = super::discovery::discover_models("agy").await;
            update_live_catalog(&models);
        }
    }
}

/// Checks whether `canonical_or_base` appears in the cached live `agy models` list.
pub fn is_in_live_catalog(canonical_or_base: &str) -> bool {
    let low = canonical_or_base.trim().to_ascii_lowercase();
    with_live_catalog(|cat| {
        cat.is_some_and(|c| {
            c.iter().any(|e| {
                e.eq_ignore_ascii_case(&low) || strip_effort_suffix(e).0.eq_ignore_ascii_case(&low)
            })
        })
    })
}

/// Normalizes shorthand or legacy Antigravity model aliases to canonical IDs.
pub fn normalize_model_id(raw: &str) -> &str {
    match raw {
        "opus" | "claude-opus" | "opus-5-5" | "opus5.5" => "claude-opus-5-5",
        "sonnet" | "claude-sonnet" | "sonnet-5-5" | "sonnet5.5" => "claude-sonnet-5-5",
        "flash" | "gemini" | "gemini-2.5-flash" | "gemini-3-flash" | "gemini-3.0-flash"
        | "gemini-3.5-flash" | "gemini-3.6-flash" | "gemini-3.7-flash" | "gemini-3.8-flash" => {
            "gemini-3.8-flash"
        }
        "argon" | "gemini-argon" | "gemini-4-argon" | "pro" | "gemini-2.5-pro"
        | "gemini-3.0-pro" | "gemini-3.1-pro" => "gemini-4-argon",
        "carbon" | "gemini-carbon" | "gemini-4-carbon" => "gemini-4-carbon",
        "hydrogen" | "gemini-hydrogen" | "gemini-4-hydrogen" => "gemini-4-hydrogen",
        "orchestrator" | "orchestrator-carbon" | "gemini-orchestrator"
        | "gemini-pro-orchestrator" => "gemini-pro-orchestrator",
        "orchestrator-argon" | "gemini-pro-orchestrator-argon" => "gemini-pro-orchestrator-argon",
        "gpt-oss" | "gpt-oss-120b" | "gpt-oss-120b-medium" => "gpt-oss-120b-medium",
        other => other,
    }
}

fn catalog_efforts_for_base(canonical_base: &str, catalog: &[String]) -> Vec<String> {
    let mut efforts = Vec::new();
    for entry in catalog {
        let (base, eff_opt) = strip_effort_suffix(entry);
        if base.eq_ignore_ascii_case(canonical_base)
            && let Some(eff) = eff_opt
            && !efforts.iter().any(|e| e == eff)
        {
            efforts.push(eff.to_string());
        }
    }
    efforts
}

/// Resolves `raw` to its canonical Antigravity model ID, preserving single-fixed-variant
/// catalog entries (such as `gpt-oss-120b-medium` or future 1-variant models) intact.
pub fn canonicalize_model_with_catalog(raw: &str, catalog: Option<&[String]>) -> String {
    let low = raw.trim().to_ascii_lowercase();
    if low.is_empty() || low == "antigravity-default" {
        return "antigravity-default".to_string();
    }
    let exact_alias = normalize_model_id(&low);
    if exact_alias != low {
        return exact_alias.to_string();
    }
    let (base, _) = strip_effort_suffix(&low);
    let norm = normalize_model_id(base);
    if let Some(cat) = catalog.filter(|c| !c.is_empty()) {
        if let Some(found) = cat.iter().find(|m| m.eq_ignore_ascii_case(&low))
            && catalog_efforts_for_base(base, cat).len() <= 1
        {
            return found.to_ascii_lowercase();
        }
        let siblings: Vec<&String> = cat
            .iter()
            .filter(|m| strip_effort_suffix(m).0.eq_ignore_ascii_case(norm))
            .collect();
        if siblings.len() == 1 && !cat.iter().any(|m| m.eq_ignore_ascii_case(norm)) {
            return siblings[0].to_ascii_lowercase();
        }
    }
    norm.to_string()
}

/// Resolves `raw` against the cached live `agy models` catalog (if populated) and static aliases.
pub fn canonicalize_model(raw: &str) -> String {
    with_live_catalog(|cat| canonicalize_model_with_catalog(raw, cat))
}

/// Returns the ordered list of reasoning effort levels supported by `model` under
/// an optional discovered `catalog` snapshot (empty vector for effortless or fixed-tier models).
pub fn supported_efforts_with_catalog(model: &str, catalog: Option<&[String]>) -> Vec<String> {
    let canonical = canonicalize_model_with_catalog(model, catalog);
    if canonical == "antigravity-default" {
        return Vec::new();
    }

    if let Some(cat) = catalog.filter(|c| !c.is_empty()) {
        let (c_base, _) = strip_effort_suffix(&canonical);
        let cat_efforts = catalog_efforts_for_base(c_base, cat);
        if cat_efforts.len() >= 2 {
            return cat_efforts;
        }
        if cat
            .iter()
            .any(|m| m.eq_ignore_ascii_case(&canonical) || strip_effort_suffix(m).0.eq_ignore_ascii_case(c_base))
        {
            return Vec::new();
        }
    }

    if canonical.contains("orchestrator") || canonical.starts_with("gpt-oss-120b") {
        Vec::new()
    } else if canonical.contains("hydrogen") {
        vec!["high".into(), "xhigh".into(), "medium".into(), "low".into()]
    } else {
        vec!["high".into(), "medium".into(), "low".into()]
    }
}

/// Returns the ordered list of reasoning effort levels supported by `model`,
/// consulting the live `agy models` cache when populated.
pub fn supported_efforts(model: &str) -> Vec<String> {
    with_live_catalog(|cat| supported_efforts_with_catalog(model, cat))
}

/// Returns `true` if `model` accepts configurable `--effort` / `-<effort>` selection in `agy`.
pub fn supports_effort(model: &str) -> bool {
    !supported_efforts(model).is_empty()
}

fn clamp_to_supported(req: &str, supported: &[String]) -> Option<String> {
    if supported.is_empty() {
        return None;
    }
    let norm = req.trim().to_ascii_lowercase();
    if supported.iter().any(|s| s == &norm) {
        return Some(norm);
    }
    let candidates: &[&str] = match norm.as_str() {
        "minimal" | "low" => &["low", "medium", "high", "xhigh", "max"],
        "medium" => &["medium", "high", "low", "xhigh", "max"],
        "high" => &["high", "medium", "xhigh", "low", "max"],
        "xhigh" | "max" | "ultra" => &["xhigh", "max", "high", "medium", "low"],
        _ => &[],
    };
    for &cand in candidates {
        if supported.iter().any(|s| s == cand) {
            return Some(cand.to_string());
        }
    }
    supported.first().cloned()
}

/// Clamps `effort` to the supported levels for `model`, returning `None` if `model` is effortless.
pub fn clamp_effort_for_model(model: &str, effort: &str) -> Option<String> {
    clamp_to_supported(effort, &supported_efforts(model))
}

fn default_effort_for_canonical(canonical: &str, supported: &[String]) -> Option<String> {
    if supported.is_empty() {
        return None;
    }
    ["claude-", "opus", "sonnet", "pro", "argon", "carbon", "hydrogen"]
        .iter()
        .any(|k| canonical.contains(k))
        .then(|| clamp_to_supported("high", supported))
        .flatten()
}

/// Pure deterministic resolver mapping `(model, effort)` and an optional `agy models`
/// catalog snapshot to `(canonical_model, valid_cli_effort)`.
pub fn resolve_cli_model_effort_with_catalog(
    model: Option<&str>,
    effort: Option<&str>,
    catalog: Option<&[String]>,
) -> (Option<String>, Option<String>) {
    let Some(raw_m) = model.map(str::trim).filter(|s| !s.is_empty()) else {
        return (None, None);
    };
    let canonical = canonicalize_model_with_catalog(raw_m, catalog);
    if canonical == "antigravity-default" {
        return (None, None);
    }
    let supported = supported_efforts_with_catalog(&canonical, catalog);
    if supported.is_empty() {
        return (Some(canonical), None);
    }
    let (_, suffix_eff) = strip_effort_suffix(raw_m);
    let raw_eff = effort.map(str::trim).filter(|&e| !e.is_empty()).or(suffix_eff);
    let resolved_eff = match raw_eff {
        Some(r) => clamp_to_supported(r, &supported),
        None => default_effort_for_canonical(&canonical, &supported),
    };
    (Some(canonical), resolved_eff)
}

/// Resolves `(model, effort)` against the live `agy models` catalog (if available)
/// and static `agy 1.5.9` rules.
pub fn resolve_cli_model_effort(
    model: Option<&str>,
    effort: Option<&str>,
) -> (Option<String>, Option<String>) {
    with_live_catalog(|cat| resolve_cli_model_effort_with_catalog(model, effort, cat))
}

/// Formats the exact `/model <target>` identifier required by an authenticated `agy` PTY session,
/// returning `None` when `model` is empty or `"antigravity-default"`.
pub fn format_pty_model_target(
    model: &str,
    sess_effort: Option<&str>,
    cfg_effort: Option<&str>,
) -> Option<String> {
    let trimmed = model.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("antigravity-default") {
        return None;
    }
    let eff_hint = sess_effort.filter(|s| !s.is_empty()).or_else(|| cfg_effort.filter(|s| !s.is_empty()));
    let (resolved_m, resolved_eff) = resolve_cli_model_effort(Some(trimmed), eff_hint);
    let base_m = resolved_m?;
    let supported = supported_efforts(&base_m);
    if supported.is_empty() {
        return Some(base_m);
    }
    let eff = resolved_eff.or_else(|| supported.into_iter().next())?;
    Some(format!("{base_m}-{eff}"))
}

/// Static fallback Antigravity (`agy 1.5.9`) model catalog when live `agy models` fails.
pub fn fallback_models() -> Vec<String> {
    [
        "gemini-4-carbon-high", "gemini-4-carbon-medium", "gemini-4-carbon-low",
        "gemini-4-argon-high", "gemini-4-argon-medium", "gemini-4-argon-low",
        "gemini-4-hydrogen-high", "gemini-4-hydrogen-xhigh", "gemini-4-hydrogen-medium", "gemini-4-hydrogen-low",
        "gemini-pro-orchestrator", "gemini-pro-orchestrator-argon",
        "gemini-3.8-flash-high", "gemini-3.8-flash-medium", "gemini-3.8-flash-low",
        "claude-opus-5-5-high", "claude-opus-5-5-medium", "claude-opus-5-5-low",
        "claude-sonnet-5-5-high", "claude-sonnet-5-5-medium", "claude-sonnet-5-5-low",
        "gpt-oss-120b-medium", "antigravity-default",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}
