//! # Telegram Model Directives, Overrides, and Argument Tokenization
//!
//! ## Overview
//! Parses provider-aware inline `@model` / `@<alias>` user prompt directives, validates
//! explicit `--effort` / `--effort=<val>` flags consistently across `@model` and `/model`,
//! and applies inline model overrides to persisted sessions and warm PTYs.
//!
//! ## Search Tags
//! #model-directive, #session-model, #orchestrator-alias, #effort-clamping

use crate::cli::antigravity::AntigravityCli;
use crate::session::data::SessionData;
use super::commands_model::{is_valid_effort_level, normalize_model_for_provider};

/// Outcome of inspecting user text for a leading `@model` or `@<alias>` directive.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DirectiveOutcome<'a> {
    /// No leading model directive present in `text`.
    None,
    /// Leading `@model` directive contained an empty or unsupported `--effort` flag.
    InvalidEffort,
    /// Leading `@model` directive named a model incompatible with the active provider.
    IncompatibleModel(String),
    /// Resolved `(canonical_model, resolved_effort, remaining_prompt)`.
    Resolved(String, Option<String>, &'a str),
}

/// Returns `true` if `model` supports reasoning effort selection under `provider`.
pub(crate) fn session_supports_effort(provider: &str, model: &str) -> bool {
    let trimmed = model.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("antigravity-default") {
        return false;
    }
    let prov = SessionData::canonical_provider(provider).unwrap_or("antigravity");
    let resolved = SessionData::canonicalize_model_for_provider(prov, trimmed);
    let m = resolved.as_deref().unwrap_or(trimmed);
    match prov {
        "claude" => crate::cli::claude::supports_claude_effort(m),
        "codex" => crate::cli::codex::supports_codex_effort(m),
        _ => AntigravityCli::supports_effort(m),
    }
}

/// Clamps `raw_effort` to a valid effort level for `(provider, model)`, or `None` if effortless.
pub(crate) fn clamp_session_effort(provider: &str, model: &str, raw_effort: &str) -> Option<String> {
    let prov = SessionData::canonical_provider(provider).unwrap_or("antigravity");
    if !session_supports_effort(prov, model) {
        return None;
    }
    let eff_model = SessionData::canonicalize_model_for_provider(prov, model);
    let m = eff_model.as_deref().unwrap_or(model);
    match prov {
        "claude" => crate::cli::claude::resolve_claude_model_effort(None, Some(raw_effort)).1,
        "codex" => crate::cli::codex::resolve_codex_model_effort(None, Some(raw_effort)).1,
        _ => crate::cli::antigravity::model_spec::clamp_effort_for_model(m, raw_effort),
    }
}

pub(crate) fn get_remaining_prompt(rest: &str, consumed_tokens: usize) -> &str {
    if consumed_tokens == 0 {
        return rest.trim();
    }
    let mut count = 0;
    let mut in_token = false;
    for (i, ch) in rest.char_indices() {
        if ch.is_whitespace() {
            if in_token {
                in_token = false;
                count += 1;
                if count == consumed_tokens {
                    return rest[i..].trim();
                }
            }
        } else {
            in_token = true;
        }
    }
    ""
}

fn is_shortcut_for_provider(provider: &str, dir_low: &str) -> bool {
    match SessionData::canonical_provider(provider).unwrap_or("antigravity") {
        "claude" => matches!(dir_low, "opus" | "sonnet" | "fable" | "haiku"),
        "codex" => matches!(dir_low, "astra" | "sol" | "terra" | "luna"),
        _ => matches!(
            dir_low,
            "opus" | "sonnet" | "argon" | "pro" | "carbon" | "hydrogen" | "orchestrator"
                | "orchestrator-argon" | "orchestrator-carbon" | "gemini-orchestrator"
                | "flash" | "gemini" | "gpt-oss" | "gpt-oss-120b"
        ),
    }
}

fn parse_at_model_tokens(rest: &str) -> Result<(String, Option<String>, &str), ()> {
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.is_empty() {
        return Err(());
    }
    if parts[0] == "--effort" {
        let val = parts.get(1).map(|s| s.to_ascii_lowercase()).filter(|v| is_valid_effort_level(v)).ok_or(())?;
        let raw_m = parts.get(2).ok_or(())?;
        return Ok(((*raw_m).to_string(), Some(val), get_remaining_prompt(rest, 3)));
    }
    if let Some(eq_v) = parts[0].strip_prefix("--effort=") {
        let low = eq_v.to_ascii_lowercase();
        if low.is_empty() || !is_valid_effort_level(&low) {
            return Err(());
        }
        let raw_m = parts.get(1).ok_or(())?;
        return Ok(((*raw_m).to_string(), Some(low), get_remaining_prompt(rest, 2)));
    }
    let (_, suffix_eff) = crate::cli::common::strip_effort_suffix(parts[0]);
    let mut explicit_eff = suffix_eff.map(str::to_string);
    let mut consumed = 1;
    if parts.get(1) == Some(&"--effort") {
        let val = parts.get(2).map(|s| s.to_ascii_lowercase()).filter(|v| is_valid_effort_level(v)).ok_or(())?;
        explicit_eff = Some(val);
        consumed = 3;
    } else if let Some(eq_v) = parts.get(1).and_then(|t| t.strip_prefix("--effort=")) {
        let low = eq_v.to_ascii_lowercase();
        if low.is_empty() || !is_valid_effort_level(&low) {
            return Err(());
        }
        explicit_eff = Some(low);
        consumed = 2;
    } else if let Some(p1) = parts.get(1).map(|s| s.to_ascii_lowercase()).filter(|v| is_valid_effort_level(v)) {
        explicit_eff = Some(p1);
        consumed = 2;
    }
    Ok((parts[0].to_string(), explicit_eff, get_remaining_prompt(rest, consumed)))
}

/// Resolves a leading `@model` / `@<alias>` directive for `(provider, prev_effort)`.
pub(crate) fn resolve_directive_for_provider<'a>(
    provider: &str,
    prev_effort: Option<&str>,
    text: &'a str,
) -> DirectiveOutcome<'a> {
    let t = text.trim();
    let (raw_m, explicit_eff, rem) = if let Some(r) = t.strip_prefix("@model") {
        if !r.is_empty() && !r.starts_with(char::is_whitespace) {
            return DirectiveOutcome::None;
        }
        let rest = r.trim();
        if rest.is_empty() {
            return DirectiveOutcome::None;
        }
        match parse_at_model_tokens(rest) {
            Ok(parsed) => parsed,
            Err(()) => return DirectiveOutcome::InvalidEffort,
        }
    } else if let Some(after_at) = t.strip_prefix('@') {
        if after_at.is_empty() || after_at.starts_with(char::is_whitespace) {
            return DirectiveOutcome::None;
        }
        let dir = after_at.split_whitespace().next().unwrap_or("");
        let low = dir.to_ascii_lowercase();
        if !dir.is_empty() && is_shortcut_for_provider(provider, &low) {
            (low, None, after_at[dir.len()..].trim())
        } else {
            return DirectiveOutcome::None;
        }
    } else {
        return DirectiveOutcome::None;
    };

    let prov = SessionData::canonical_provider(provider).unwrap_or("antigravity");
    let lower = raw_m.to_ascii_lowercase();
    let canonical = normalize_model_for_provider(prov, &lower);
    if !SessionData::is_model_compatible(&canonical, prov) {
        return DirectiveOutcome::IncompatibleModel(canonical);
    }
    let eff = if !session_supports_effort(prov, &canonical) {
        None
    } else if let Some(ref exp) = explicit_eff {
        clamp_session_effort(prov, &canonical, exp)
    } else {
        prev_effort.filter(|s| !s.is_empty()).and_then(|p| clamp_session_effort(prov, &canonical, p))
    };
    DirectiveOutcome::Resolved(canonical, eff, rem)
}

#[cfg(test)]
pub(crate) fn parse_model_directive_for_provider<'a>(
    provider: &str,
    text: &'a str,
) -> (Option<String>, Option<String>, &'a str) {
    match resolve_directive_for_provider(provider, None, text) {
        DirectiveOutcome::Resolved(m, e, r) => (Some(m), e, r),
        DirectiveOutcome::None | DirectiveOutcome::InvalidEffort | DirectiveOutcome::IncompatibleModel(_) => {
            (None, None, text.trim())
        }
    }
}

#[cfg(test)]
pub(crate) fn parse_model_directive(text: &str) -> (Option<String>, Option<String>, &str) {
    parse_model_directive_for_provider("antigravity", text)
}

/// Validates and tokenizes `/model <args>` into `Ok((raw_model_token, explicit_effort))`
/// or `Err(())` when an explicit `--effort` / `--effort=<val>` flag is malformed or invalid.
pub(crate) fn tokenize_model_command_args(args: &str) -> Result<(String, Option<String>), ()> {
    let tokens: Vec<&str> = args.split_whitespace().collect();
    let mut rem = Vec::new();
    let mut raw_eff = None;
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == "--effort" {
            let v = tokens.get(i + 1).map(|s| s.to_ascii_lowercase()).filter(|v| is_valid_effort_level(v)).ok_or(())?;
            raw_eff = Some(v);
            i += 2;
        } else if let Some(eq_v) = tokens[i].strip_prefix("--effort=") {
            let low = eq_v.to_ascii_lowercase();
            if low.is_empty() || !is_valid_effort_level(&low) {
                return Err(());
            }
            raw_eff = Some(low);
            i += 1;
        } else {
            rem.push(tokens[i]);
            i += 1;
        }
    }
    if rem.is_empty() || rem.len() > 2 {
        return Err(());
    }
    let model_part = if rem.len() == 2 {
        if raw_eff.is_some() || !is_valid_effort_level(&rem[1].to_ascii_lowercase()) {
            return Err(());
        }
        raw_eff = Some(rem[1].to_ascii_lowercase());
        rem[0].to_string()
    } else {
        rem[0].to_string()
    };
    let (_, suf_eff) = crate::cli::common::strip_effort_suffix(&model_part);
    if raw_eff.is_none()
        && let Some(eff) = suf_eff
    {
        raw_eff = Some(eff.to_string());
    }
    Ok((model_part, raw_eff))
}

/// Parses `/model <args>` arguments into `(canonical_model, valid_effort)` for `provider`.
pub(crate) fn parse_model_effort_args_for_provider(
    provider: &str,
    args: &str,
    cfg_effort: Option<&str>,
) -> (String, Option<String>) {
    let prov = SessionData::canonical_provider(provider).unwrap_or("antigravity");
    let (model_part, raw_eff) = tokenize_model_command_args(args)
        .unwrap_or_else(|()| (args.trim().to_string(), None));
    let lower_model = model_part.to_ascii_lowercase();
    let cfg_hint = cfg_effort.map(str::trim).filter(|s| !s.is_empty());
    match prov {
        "claude" => {
            let canonical = normalize_model_for_provider("claude", &lower_model);
            if !crate::cli::claude::supports_claude_effort(&canonical) {
                return (canonical, None);
            }
            let eff = match raw_eff.as_deref().or(cfg_hint) {
                Some(e) => clamp_session_effort("claude", &canonical, e),
                None => Some("high".to_string()),
            };
            (canonical, eff)
        }
        "codex" => {
            let canonical = normalize_model_for_provider("codex", &lower_model);
            let eff = raw_eff.as_deref().or(cfg_hint).and_then(|e| clamp_session_effort("codex", &canonical, e));
            (canonical, eff)
        }
        _ => {
            if lower_model == "antigravity-default" {
                return ("antigravity-default".to_string(), None);
            }
            let (m, eff) = AntigravityCli::resolve_cli_model_effort(
                Some(&lower_model),
                raw_eff.as_deref().or(cfg_hint),
            );
            (m.unwrap_or_else(|| crate::cli::antigravity::model_spec::canonicalize_model(&lower_model)), eff)
        }
    }
}
