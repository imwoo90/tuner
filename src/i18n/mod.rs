//! # Internationalization (i18n) Module
//!
//! ## Overview
//! Loads TOML translation catalogs into static registries for thread-safe and task-local
//! translation lookup across chat messages.
//!
//! ## Submodules
//! - [`loader`] (`loader.rs`): Loads and flattens translation strings from locale TOML directories.
//!
//! ## Search Tags
//! #i18n-module, #translation-store, #locale-selector, #macro-lookup

pub mod loader;

pub use loader::{TranslationStore, find_locales_dir, flatten, load_toml, load_language, format_string};

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// Static list of supported languages.
pub static LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("de", "Deutsch"),
    ("nl", "Nederlands"),
    ("es", "Español"),
    ("fr", "Français"),
    ("id", "Bahasa Indonesia"),
    ("pt", "Português"),
    ("ru", "Русский"),
    ("ko", "한국어"),
];

static REGISTRY: OnceLock<HashMap<String, TranslationStore>> = OnceLock::new();
static GLOBAL_ACTIVE_LANG: RwLock<String> = RwLock::new(String::new());

tokio::task_local! {
    /// Task-local active locale code override.
    pub static TASK_ACTIVE_LANG: String;
}

/// Retrieve the static global registry, initializing it if necessary.
fn get_registry() -> &'static HashMap<String, TranslationStore> {
    REGISTRY.get_or_init(|| {
        let root = find_locales_dir();
        let mut map = HashMap::new();
        for &(code, _) in LANGUAGES {
            let store = TranslationStore::new_with_root(code, &root);
            map.insert(code.to_string(), store);
        }
        map
    })
}

/// Helper function to retrieve the active language, falling back to global settings.
fn get_active_language() -> String {
    if let Ok(task_lang) = TASK_ACTIVE_LANG.try_with(|l| l.clone()) {
        return task_lang;
    }
    match GLOBAL_ACTIVE_LANG.read() {
        Ok(guard) => {
            if !guard.is_empty() {
                return guard.clone();
            }
        }
        Err(poisoned) => {
            let guard = poisoned.into_inner();
            if !guard.is_empty() {
                return guard.clone();
            }
        }
    }
    "en".to_string()
}

/// Initialize the global active language.
///
/// # Examples
/// ```no_run
/// tuner::i18n::init("en");
/// assert!(!tuner::i18n::t("bot.startup_complete", &[]).is_empty());
/// ```
pub fn init(language: &str) {
    let lang = if LANGUAGES.iter().any(|(code, _)| *code == language) {
        language
    } else {
        "en"
    };
    match GLOBAL_ACTIVE_LANG.write() {
        Ok(mut guard) => {
            *guard = lang.to_string();
        }
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            *guard = lang.to_string();
        }
    }
}

/// Translate a chat string for an explicit language code.
pub fn t_for_lang(lang: &str, key: &str, args: &[(&str, &str)]) -> String {
    let registry = get_registry();
    let Some(store) = registry.get(lang).or_else(|| registry.get("en")) else {
        return key.to_string();
    };
    store.chat(key, args)
}

/// Translate a chat string.
///
/// # Examples
/// ```no_run
/// tuner::i18n::init("en");
/// let val = tuner::i18n::t("bot.startup_complete", &[]);
/// assert!(val.contains("System restart"));
/// ```
pub fn t(key: &str, args: &[(&str, &str)]) -> String {
    let lang = get_active_language();
    t_for_lang(&lang, key, args)
}

/// Get the active language code.
pub fn get_language() -> String {
    get_active_language()
}

#[macro_export]
/// Helper macro `t!` (t).
macro_rules! t {
    ($key:expr) => {
        $crate::i18n::t($key, &[])
    };
    ($key:expr, $($name:ident = $val:expr),* $(,)?) => {
        $crate::i18n::t($key, &[
            $((stringify!($name), &format!("{}", $val) as &str)),*
        ])
    };
}
