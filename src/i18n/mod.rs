pub mod loader;
pub mod check;

pub use loader::{TranslationStore, find_locales_dir, flatten, load_toml, load_language, format_string};

use std::sync::RwLock;

pub static LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("de", "Deutsch"),
    ("nl", "Nederlands"),
    ("es", "Español"),
    ("fr", "Français"),
    ("id", "Bahasa Indonesia"),
    ("pt", "Português"),
    ("ru", "Русский"),
];

static STORE: RwLock<Option<TranslationStore>> = RwLock::new(None);

pub fn init(language: &str) {
    let lang = if LANGUAGES.iter().any(|(code, _)| *code == language) {
        language
    } else {
        "en"
    };
    let store = TranslationStore::new(lang);
    let mut write_guard = STORE.write().unwrap();
    *write_guard = Some(store);
}

fn get_or_init_store() -> std::sync::RwLockReadGuard<'static, Option<TranslationStore>> {
    {
        let read_guard = STORE.read().unwrap();
        if read_guard.is_some() {
            return read_guard;
        }
    }
    init("en");
    STORE.read().unwrap()
}

pub fn t(key: &str, args: &[(&str, &str)]) -> String {
    let guard = get_or_init_store();
    guard.as_ref().unwrap().chat(key, args)
}

pub fn t_rich(key: &str, args: &[(&str, &str)]) -> String {
    let guard = get_or_init_store();
    guard.as_ref().unwrap().cli(key, args)
}

pub fn t_cmd(key: &str) -> String {
    let guard = get_or_init_store();
    guard.as_ref().unwrap().cmd(key)
}

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

pub fn get_language() -> String {
    let guard = get_or_init_store();
    guard.as_ref().unwrap().language.clone()
}

pub fn get_store() -> TranslationStore {
    let guard = get_or_init_store();
    guard.as_ref().unwrap().clone()
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
