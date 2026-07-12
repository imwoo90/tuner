//! # i18n Loader Module
//!
//! Handles locating locale files, parsing TOML translations, flattening structures,
//! and doing placeholder substitution.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Find the directory where translation locales are stored.
pub fn find_locales_dir() -> PathBuf {
    if let Ok(val) = std::env::var("WOODUCTOR_I18N_DIR") {
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

/// Load the entire translation suite (chat, cli/wizard, commands) for a language.
pub fn load_language(root: &Path, lang: &str) -> (HashMap<String, String>, HashMap<String, String>, HashMap<String, String>) {
    if !is_safe_lang(lang) {
        return (HashMap::new(), HashMap::new(), HashMap::new());
    }
    let lang_dir = root.join(lang);
    let chat = load_toml(&lang_dir.join("chat.toml")).unwrap_or_default();
    let mut cli = load_toml(&lang_dir.join("cli.toml")).unwrap_or_default();
    let wizard = load_toml(&lang_dir.join("wizard.toml")).unwrap_or_default();
    for (k, v) in wizard {
        cli.insert(format!("wizard.{}", k), v);
    }
    let cmd = load_toml(&lang_dir.join("commands.toml")).unwrap_or_default();
    (chat, cli, cmd)
}

/// Represents a collection of localized strings and their English fallbacks.
#[derive(Clone, Debug)]
pub struct TranslationStore {
    pub language: String,
    pub en_chat: HashMap<String, String>,
    pub en_cli: HashMap<String, String>,
    pub en_cmd: HashMap<String, String>,
    pub primary_chat: Option<HashMap<String, String>>,
    pub primary_cli: Option<HashMap<String, String>>,
    pub primary_cmd: Option<HashMap<String, String>>,
}

impl TranslationStore {
    /// Create a new TranslationStore using the default locales path.
    pub fn new(language: &str) -> Self {
        let root = find_locales_dir();
        Self::new_with_root(language, &root)
    }

    /// Create a new TranslationStore with a specific locales path.
    pub fn new_with_root(language: &str, root: &Path) -> Self {
        let (en_chat, en_cli, en_cmd) = load_language(root, "en");
        let language = if is_safe_lang(language) { language } else { "en" };
        let (primary_chat, primary_cli, primary_cmd) = if language == "en" {
            (None, None, None)
        } else {
            let (chat, cli, cmd) = load_language(root, language);
            (Some(chat), Some(cli), Some(cmd))
        };
        Self {
            language: language.to_string(),
            en_chat,
            en_cli,
            en_cmd,
            primary_chat,
            primary_cli,
            primary_cmd,
        }
    }

    /// Retrieve a chat translation.
    pub fn chat(&self, key: &str, args: &[(&str, &str)]) -> String {
        self.resolve(self.primary_chat.as_ref(), &self.en_chat, key, args)
    }

    /// Retrieve a CLI translation.
    pub fn cli(&self, key: &str, args: &[(&str, &str)]) -> String {
        self.resolve(self.primary_cli.as_ref(), &self.en_cli, key, args)
    }

    /// Retrieve a command translation.
    pub fn cmd(&self, key: &str) -> String {
        let raw = self.primary_cmd.as_ref()
            .and_then(|m| m.get(key))
            .or_else(|| self.en_cmd.get(key));
        match raw {
            Some(val) => val.clone(),
            None => format!("[MISSING: {}]", key),
        }
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

    pub fn all_chat_keys(&self) -> HashSet<String> {
        self.en_chat.keys().cloned().collect()
    }

    pub fn all_cli_keys(&self) -> HashSet<String> {
        self.en_cli.keys().cloned().collect()
    }

    pub fn all_cmd_keys(&self) -> HashSet<String> {
        self.en_cmd.keys().cloned().collect()
    }

    pub fn lang_chat_keys(&self) -> HashSet<String> {
        match &self.primary_chat {
            Some(m) => m.keys().cloned().collect(),
            None => self.en_chat.keys().cloned().collect(),
        }
    }

    pub fn lang_cli_keys(&self) -> HashSet<String> {
        match &self.primary_cli {
            Some(m) => m.keys().cloned().collect(),
            None => self.en_cli.keys().cloned().collect(),
        }
    }

    pub fn lang_cmd_keys(&self) -> HashSet<String> {
        match &self.primary_cmd {
            Some(m) => m.keys().cloned().collect(),
            None => self.en_cmd.keys().cloned().collect(),
        }
    }
}

/// Format a translation string by replacing placeholders like `{name}` or `{first.name}`.
/// Performs best-effort substitution when some arguments are missing.
pub fn format_string(_key: &str, raw: &str, args: &[(&str, &str)]) -> String {
    use regex::Regex;
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\{([\w.-]+)\}").unwrap());

    if args.is_empty() {
        return raw.to_string();
    }

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
