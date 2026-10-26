//! # i18n Loader Module
//!
//! ## Overview
//! Handles locating locale files, parsing TOML translations, flattening structures,
//! and doing placeholder substitution.
//!
//! ## Search Tags
//! #loader

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Find the directory where translation locales are stored.
pub fn find_locales_dir() -> PathBuf {
    if let Ok(val) = std::env::var("TUNER_I18N_DIR") {
        let p = PathBuf::from(val);
        if p.exists() {
            return p;
        }
    }
    for p in &[
        "src/i18n/locales",
        "i18n/locales",
        "../src/i18n/locales",
    ] {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return pb;
        }
    }
    PathBuf::from("src/i18n/locales")
}

/// Recursively flattens a nested TOML value into a dot-notation key map.
pub fn flatten(value: &toml::Value, prefix: &str, map: &mut HashMap<String, String>) {
    match value {
        toml::Value::Table(table) => {
            for (k, v) in table {
                let full_key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{}.{}", prefix, k)
                };
                flatten(v, &full_key, map);
            }
        }
        toml::Value::String(s) => {
            map.insert(prefix.to_string(), s.clone());
        }
        _ => {
            let s = match value {
                toml::Value::Integer(i) => i.to_string(),
                toml::Value::Float(f) => f.to_string(),
                toml::Value::Boolean(b) => b.to_string(),
                toml::Value::Datetime(dt) => dt.to_string(),
                _ => value.to_string(),
            };
            map.insert(prefix.to_string(), s);
        }
    }
}

/// Load and parse a TOML file into a flat key-value map.
///
/// # Examples
/// ```no_run
/// use std::path::Path;
/// let result = tuner::i18n::loader::load_toml(Path::new("en/chat.toml"));
/// ```
pub fn load_toml(path: &Path) -> anyhow::Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    let content = std::fs::read_to_string(path)?;
    let value = content.parse::<toml::Value>()?;
    flatten(&value, "", &mut map);
    Ok(map)
}

/// Load the entire translation suite (chat, cli/wizard, commands) for a language.
fn is_safe_lang(lang: &str) -> bool {
    if lang.is_empty() {
        return false;
    }
    lang.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn embedded_chat_toml(lang: &str) -> Option<&'static str> {
    match lang {
        "en" => Some(include_str!("locales/en/chat.toml")),
        "de" => Some(include_str!("locales/de/chat.toml")),
        "nl" => Some(include_str!("locales/nl/chat.toml")),
        "es" => Some(include_str!("locales/es/chat.toml")),
        "fr" => Some(include_str!("locales/fr/chat.toml")),
        "id" => Some(include_str!("locales/id/chat.toml")),
        "pt" => Some(include_str!("locales/pt/chat.toml")),
        "ru" => Some(include_str!("locales/ru/chat.toml")),
        "ko" => Some(include_str!("locales/ko/chat.toml")),
        _ => None,
    }
}

/// Load the chat translation catalog for a language, falling back to embedded assets when running standalone.
pub fn load_language(root: &Path, lang: &str) -> HashMap<String, String> {
    if !is_safe_lang(lang) {
        return HashMap::new();
    }
    let chat_path = root.join(lang).join("chat.toml");
    if chat_path.is_file() {
        return load_toml(&chat_path).unwrap_or_default();
    }
    if !root.is_dir()
        && let Some(raw) = embedded_chat_toml(lang)
            && let Ok(val) = raw.parse::<toml::Value>() {
                let mut map = HashMap::new();
                flatten(&val, "", &mut map);
                return map;
            }
    HashMap::new()
}

/// Represents a collection of localized chat strings and their English fallbacks.
#[derive(Clone, Debug)]
pub struct TranslationStore {
    /// Language.
    pub language: String,
    /// En chat.
    pub en_chat: HashMap<String, String>,
    /// Primary chat.
    pub primary_chat: Option<HashMap<String, String>>,
}

impl TranslationStore {
    /// Create a new TranslationStore using the default locales path.
    pub fn new(language: &str) -> Self {
        let root = find_locales_dir();
        Self::new_with_root(language, &root)
    }

    /// Create a new TranslationStore with a specific locales path.
    pub fn new_with_root(language: &str, root: &Path) -> Self {
        let en_chat = load_language(root, "en");
        let language = if is_safe_lang(language) { language } else { "en" };
        let primary_chat = if language == "en" {
            None
        } else {
            Some(load_language(root, language))
        };
        Self {
            language: language.to_string(),
            en_chat,
            primary_chat,
        }
    }

    /// Retrieve a chat translation.
    pub fn chat(&self, key: &str, args: &[(&str, &str)]) -> String {
        self.resolve(self.primary_chat.as_ref(), &self.en_chat, key, args)
    }

    fn resolve(
        &self,
        primary: Option<&HashMap<String, String>>,
        fallback: &HashMap<String, String>,
        key: &str,
        args: &[(&str, &str)],
    ) -> String {
        let raw = primary
            .and_then(|m| m.get(key))
            .filter(|s| !s.is_empty())
            .or_else(|| fallback.get(key));
        match raw {
            Some(raw_str) => format_string(key, raw_str, args),
            None => format!("[MISSING: {}]", key),
        }
    }

    /// Executes `all_chat_keys` (all chat keys).
    pub fn all_chat_keys(&self) -> HashSet<String> {
        self.en_chat.keys().cloned().collect()
    }

    /// Executes `lang_chat_keys` (lang chat keys).
    pub fn lang_chat_keys(&self) -> HashSet<String> {
        match &self.primary_chat {
            Some(m) => m.keys().cloned().collect(),
            None => self.en_chat.keys().cloned().collect(),
        }
    }
}

/// Format a translation string by replacing placeholders like `{name}` or `{first.name}`.
/// Performs best-effort substitution when some arguments are missing.
pub fn format_string(_key: &str, raw: &str, args: &[(&str, &str)]) -> String {
    use regex::Regex;
    use std::sync::OnceLock;
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    if args.is_empty() {
        return raw.to_string();
    }
    let Some(re) = RE.get_or_init(|| Regex::new(r"\{([\w.-]+)\}").ok()) else {
        return raw.to_string();
    };

    let result = re.replace_all(raw, |caps: &regex::Captures| {
        let ph_name = &caps[1];
        if let Some((_, val)) = args.iter().find(|(k, _)| *k == ph_name) {
            val.to_string()
        } else {
            caps[0].to_string()
        }
    });

    result.into_owned()
}
