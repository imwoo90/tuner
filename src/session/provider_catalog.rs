//! # Session Provider & Model Catalog Normalization (`provider_catalog.rs`)
//!
//! ## Overview
//! Validates cross-provider model compatibility, resolves provider-specific default models,
//! clamps effort overrides, and applies `/provider` switches onto [`SessionData`].
//!
//! ## Search Tags
//! #provider-catalog, #model-compatibility, #effort-clamping

use super::data::SessionData;

impl SessionData {
    /// Normalizes a provider alias (`agy`, `antigravity`, `claude`, `codex`) into its canonical provider name.
    pub fn canonical_provider(raw: &str) -> Option<&'static str> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "antigravity" | "agy" => Some("antigravity"),
            "claude" | "claude-code" => Some("claude"),
            "codex" | "openai" => Some("codex"),
            _ => None,
        }
    }

    /// Resolves `raw` (falling back to `fallback`, then `"antigravity"`) into a canonical provider name.
    pub(crate) fn effective_provider(raw: &str, fallback: &str) -> &'static str {
        let candidate = if raw.trim().is_empty() { fallback } else { raw };
        Self::canonical_provider(candidate).unwrap_or("antigravity")
    }

    /// Returns `true` if `model` (including provider aliases and effort suffixes) belongs to `provider`.
    pub(crate) fn is_model_compatible(model: &str, provider: &str) -> bool {
        Self::canonicalize_model_for_provider(provider, model).is_some()
    }

    fn canonicalize_claude_base(base: &str) -> Option<String> {
        let canon = match base {
            "fable" | "claude-fable" => "claude-fable-5",
            "sonnet" | "claude-sonnet" => "claude-sonnet-5",
            "opus" | "claude-opus" => "claude-opus-4-8",
            "haiku" | "claude-haiku" => "claude-haiku-4-5",
            other => other,
        };
        let ok = crate::cli::claude::CLAUDE_MODELS.contains(&canon)
            || (canon.starts_with("claude-")
                && !canon.ends_with("-5-5")
                && !canon.starts_with("claude-opus-5-5")
                && !canon.starts_with("claude-sonnet-5-5"));
        ok.then(|| canon.to_string())
    }

    fn canonicalize_codex_base(base: &str) -> Option<String> {
        let canon = match base {
            "astra" => "gpt-6-astra",
            "sol" => "gpt-6.1-sol",
            "terra" => "gpt-6.1-terra",
            "luna" => "gpt-6-luna",
            other => other,
        };
        let ok = crate::cli::codex::CODEX_MODELS.contains(&canon)
            || ((canon.starts_with("gpt-5")
                || canon.starts_with("gpt-6")
                || canon.starts_with("o1")
                || canon.starts_with("o3")
                || canon.starts_with("o4")
                || canon.starts_with("codex"))
                && !canon.starts_with("gpt-oss"));
        ok.then(|| canon.to_string())
    }

    /// Canonicalizes a compatible model identifier or provider alias into its lowercase base ID.
    pub fn canonicalize_model_for_provider(provider: &str, raw: &str) -> Option<String> {
        let prov = Self::canonical_provider(provider).unwrap_or("antigravity");
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) {
            return None;
        }
        let low = trimmed.to_ascii_lowercase();
        let (base, _) = crate::cli::common::strip_effort_suffix(&low);
        match prov {
            "claude" => Self::canonicalize_claude_base(base),
            "codex" => Self::canonicalize_codex_base(base),
            _ => {
                let m = crate::cli::antigravity::model_spec::canonicalize_model(&low);
                let ok = m == "antigravity-default"
                    || m.starts_with("gemini-")
                    || m.starts_with("gpt-oss-")
                    || matches!(m.as_str(), "claude-opus-5-5" | "claude-sonnet-5-5")
                    || crate::cli::antigravity::model_spec::is_in_live_catalog(&m);
                ok.then_some(m)
            }
        }
    }

    /// Resolves a provider-compatible default model for `provider`, keeping `configured_model`
    /// when valid for `provider` and falling back to the provider's native default otherwise.
    pub fn default_model_for_provider(provider: &str, configured_model: Option<&str>) -> String {
        let prov = Self::canonical_provider(provider).unwrap_or("antigravity");
        if let Some(m) = configured_model.and_then(|raw| Self::canonicalize_model_for_provider(prov, raw)) {
            return m;
        }
        match prov {
            "claude" => crate::cli::claude::DEFAULT_CLAUDE_MODEL.to_string(),
            "codex" => crate::cli::codex::DEFAULT_CODEX_MODEL.to_string(),
            _ => "antigravity-default".to_string(),
        }
    }

    pub(crate) fn normalize_effort_for_provider(&mut self, target_provider: &str) {
        let prov = Self::canonical_provider(target_provider).unwrap_or("antigravity");
        match prov {
            "claude" => {
                if !crate::cli::claude::supports_claude_effort(&self.model) {
                    self.effort = None;
                } else if let Some(ref eff) = self.effort {
                    self.effort = crate::cli::claude::resolve_claude_model_effort(None, Some(eff)).1;
                }
            }
            "codex" => {
                if !crate::cli::codex::supports_codex_effort(&self.model) {
                    self.effort = None;
                } else if let Some(ref eff) = self.effort {
                    self.effort = crate::cli::codex::resolve_codex_model_effort(None, Some(eff)).1;
                }
            }
            _ => {
                if !crate::cli::antigravity::AntigravityCli::supports_effort(&self.model) {
                    self.effort = None;
                } else if let Some(ref eff) = self.effort {
                    self.effort =
                        crate::cli::antigravity::model_spec::clamp_effort_for_model(&self.model, eff);
                }
            }
        }
    }

    /// Switches `self.provider` to `target_provider`, updates `self.model` and `self.effort`
    /// if incompatible, and returns a status summary.
    pub fn apply_provider_switch(&mut self, target_provider: &str) -> String {
        let prov = Self::canonical_provider(target_provider).unwrap_or("antigravity");
        self.provider = prov.to_string();
        let raw_trimmed = self.model.trim().to_string();
        let (_, suf_eff) = crate::cli::common::strip_effort_suffix(&raw_trimmed);
        if let Some(canon) = Self::canonicalize_model_for_provider(prov, &raw_trimmed) {
            if self.effort.is_none()
                && let Some(se) = suf_eff
            {
                self.effort = Some(se.to_string());
            }
            self.model = canon;
        } else {
            self.model = match prov {
                "claude" => crate::cli::claude::DEFAULT_CLAUDE_MODEL.to_string(),
                "codex" => crate::cli::codex::DEFAULT_CODEX_MODEL.to_string(),
                _ => "gemini-4-hydrogen".to_string(),
            };
        }
        self.normalize_effort_for_provider(prov);
        let existing_sid = self.get_session_id(prov);
        if !existing_sid.is_empty() {
            let short: String = existing_sid.chars().take(8).collect();
            format!(
                "🤖 [tuner] Provider switched to `{}` (model: `{}`).\nResuming existing session `{}`.",
                prov, self.model, short
            )
        } else {
            format!(
                "🤖 [tuner] Provider switched to `{}` (model: `{}`).\nA fresh session will start on your next message.",
                prov, self.model
            )
        }
    }

    /// Extracts a trailing reasoning effort suffix from `raw_model` only when `raw_model` is
    /// compatible with `provider` and its canonical base model supports configurable effort.
    pub(crate) fn extract_compatible_effort_suffix(provider: &str, raw_model: &str) -> Option<String> {
        let canon = Self::canonicalize_model_for_provider(provider, raw_model)?;
        if !crate::messenger::telegram::reply_directives::session_supports_effort(provider, &canon) {
            return None;
        }
        crate::cli::common::strip_effort_suffix(raw_model).1.map(str::to_string)
    }

    /// Resolves the configured global effort fallback (`config.effort`, else `config.model` suffix) for `provider`.
    pub(crate) fn configured_effort_fallback(
        config: &crate::config::CliConfig,
        provider: &str,
    ) -> Option<String> {
        config
            .effort
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                config
                    .model
                    .as_deref()
                    .and_then(|cm| Self::extract_compatible_effort_suffix(provider, cm))
            })
    }

    /// Resolves `(provider, canonical_model, clamped_effective_effort)` from optional `sess` + `config`.
    pub(crate) fn resolve_effective_target(
        sess: Option<&Self>,
        config: &crate::config::CliConfig,
    ) -> (String, String, Option<String>) {
        use crate::messenger::telegram::reply_directives::{clamp_session_effort, session_supports_effort};
        let raw_prov = sess.map(|s| s.provider.as_str()).unwrap_or("");
        let p = Self::effective_provider(raw_prov, &config.provider).to_string();
        let cfg_canon = config
            .model
            .as_deref()
            .and_then(|cm| Self::canonicalize_model_for_provider(&p, cm));
        let cfg_suf = config
            .model
            .as_deref()
            .and_then(|cm| Self::extract_compatible_effort_suffix(&p, cm));
        let cfg_eff = config.effort.as_deref().map(str::trim).filter(|s| !s.is_empty());

        let (self_canon, self_eff, self_suf) = match sess {
            Some(s) => (
                Self::canonicalize_model_for_provider(&p, &s.model),
                s.effort.as_deref().map(str::trim).filter(|e| !e.is_empty()),
                Self::extract_compatible_effort_suffix(&p, &s.model),
            ),
            None => (None, None, None),
        };
        let resolved_m = self_canon
            .clone()
            .unwrap_or_else(|| Self::default_model_for_provider(&p, config.model.as_deref()));
        if !session_supports_effort(&p, &resolved_m) {
            return (p, resolved_m, None);
        }
        let allow_cfg_suf = self_canon.is_none() || self_canon == cfg_canon;
        let fallback_cfg_suf = allow_cfg_suf.then_some(cfg_suf.as_deref()).flatten();
        let eff = [self_eff, self_suf.as_deref(), cfg_eff, fallback_cfg_suf]
            .into_iter()
            .flatten()
            .find_map(|e| clamp_session_effort(&p, &resolved_m, e));
        (p, resolved_m, eff)
    }

    /// Resolves `(provider, canonical_model, effective_effort)` for `/status` display.
    pub(crate) fn resolve_display_target(
        sess: Option<Self>,
        config: &crate::config::CliConfig,
    ) -> (String, String, Option<String>) {
        Self::resolve_effective_target(sess.as_ref(), config)
    }

    /// Applies this session's canonical `(provider, model, effort)` onto `cfg` for CLI execution.
    pub(crate) fn apply_to_cli_config(&self, cfg: &mut crate::config::CliConfig) {
        let (prov, resolved_m, eff) = Self::resolve_effective_target(Some(self), cfg);
        cfg.provider = prov;
        cfg.model = Some(resolved_m);
        cfg.effort = eff;
    }
}
