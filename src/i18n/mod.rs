//! # Internationalization (i18n) Module
//!
//! Provides thread-safe, read-only translation lookups across multiple locales.
//! Loads all translations into a static registry to avoid request-level overrides
//! and race conditions in concurrent environments.

pub mod loader;
pub mod check;

pub use loader::{TranslationStore, find_locales_dir, flatten, load_toml, load_language, format_string};

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use std::cell::RefCell;

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

thread_local! {
    static ACTIVE_LANG: RefCell<Option<String>> = RefCell::new(None);
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
    ACTIVE_LANG.with(|l| {
        if let Some(ref val) = *l.borrow() {
            return val.clone();
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
    })
}

/// Initialize the global active language.
///
/// # Examples
/// ```no_run
/// tuner::i18n::init("en");
/// assert_eq!(tuner::i18n::get_language(), "en");
/// ```
pub fn init(language: &str) {
    let lang = if LANGUAGES.iter().any(|(code, _)| *code == language) {
        language
    } else {
        "en"
    };
    ACTIVE_LANG.with(|l| {
        *l.borrow_mut() = Some(lang.to_string());
    });
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

/// Translate a chat string.
///
/// # Examples
/// ```no_run
/// tuner::i18n::init("en");
/// let val = tuner::i18n::t("session.error_header", &[]);
/// assert!(val.contains("Session Error"));
/// ```
pub fn t(key: &str, args: &[(&str, &str)]) -> String {
    let lang = get_active_language();
    let registry = get_registry();
    let store = registry.get(&lang).or_else(|| registry.get("en")).unwrap();
    store.chat(key, args)
}

/// Translate a rich formatting CLI string.
///
/// # Examples
/// ```no_run
/// tuner::i18n::init("en");
/// let val = tuner::i18n::t_rich("wizard.common.cancelled", &[]);
/// assert!(val.contains("cancelled"));
/// ```
pub fn t_rich(key: &str, args: &[(&str, &str)]) -> String {
    let lang = get_active_language();
    let registry = get_registry();
    let store = registry.get(&lang).or_else(|| registry.get("en")).unwrap();
    store.cli(key, args)
}

/// Translate a bot command name or description.
///
/// # Examples
/// ```no_run
/// tuner::i18n::init("en");
/// let val = tuner::i18n::t_cmd("bot.new");
/// assert!(!val.is_empty());
/// ```
pub fn t_cmd(key: &str) -> String {
    let lang = get_active_language();
    let registry = get_registry();
    let store = registry.get(&lang).or_else(|| registry.get("en")).unwrap();
    store.cmd(key)
}

/// Translate with plural form selection.
///
/// # Examples
/// ```no_run
/// tuner::i18n::init("en");
/// let val = tuner::i18n::t_plural("tasks.cancelled", 5, &[]);
/// assert!(val.contains("5 tasks"));
/// ```
pub fn t_plural(key: &str, count: i64, args: &[(&str, &str)]) -> String {
    let suffix = if count == 1 { "_one" } else { "_other" };
    let count_str = count.to_string();
    let mut new_args = Vec::with_capacity(args.len() + 1);
    new_args.push(("count", count_str.as_str()));
    for &(k, v) in args {
        if k != "count" {
            new_args.push((k, v));
        }
    }
    t(&format!("{}{}", key, suffix), &new_args)
}

/// Get the active language code.
pub fn get_language() -> String {
    get_active_language()
}

/// Get the current TranslationStore clone.
pub fn get_store() -> TranslationStore {
    let lang = get_active_language();
    let registry = get_registry();
    registry.get(&lang).or_else(|| registry.get("en")).unwrap().clone()
}

#[macro_export]
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

#[macro_export]
macro_rules! t_rich {
    ($key:expr) => {
        $crate::i18n::t_rich($key, &[])
    };
    ($key:expr, $($name:ident = $val:expr),* $(,)?) => {
        $crate::i18n::t_rich($key, &[
            $((stringify!($name), &format!("{}", $val) as &str)),*
        ])
    };
}

#[macro_export]
macro_rules! t_plural {
    ($key:expr, $count:expr) => {
        $crate::i18n::t_plural($key, $count, &[])
    };
    ($key:expr, $count:expr, $($name:ident = $val:expr),* $(,)?) => {
        $crate::i18n::t_plural($key, $count, &[
            $((stringify!($name), &format!("{}", $val) as &str)),*
        ])
    };
}
