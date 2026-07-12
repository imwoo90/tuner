use crate::i18n::{
    flatten, load_toml, TranslationStore, LANGUAGES, init, get_language, get_store, t_cmd, find_locales_dir
};
use crate::{t, t_rich, t_plural};
use std::collections::{HashMap, HashSet};
use toml::Value;

// -- _flatten ------------------------------------------------------------------

#[test]
fn test_flatten_simple() {
    let mut map = HashMap::new();
    let val: Value = toml::from_str("a = 'hello'").unwrap();
    flatten(&val, "", &mut map);
    let mut expected = HashMap::new();
    expected.insert("a".to_string(), "hello".to_string());
    assert_eq!(map, expected);
}

#[test]
fn test_flatten_nested() {
    let mut map = HashMap::new();
    let val: Value = toml::from_str("[a]\nb = 'hello'\nc = 'world'").unwrap();
    flatten(&val, "", &mut map);
    let mut expected = HashMap::new();
    expected.insert("a.b".to_string(), "hello".to_string());
    expected.insert("a.c".to_string(), "world".to_string());
    assert_eq!(map, expected);
}

#[test]
fn test_flatten_deep() {
    let mut map = HashMap::new();
    let val: Value = toml::from_str("[a.b]\nc = 'deep'").unwrap();
    flatten(&val, "", &mut map);
    let mut expected = HashMap::new();
    expected.insert("a.b.c".to_string(), "deep".to_string());
    assert_eq!(map, expected);
}

// -- load_toml ----------------------------------------------------------------

#[test]
fn test_load_toml_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let result = load_toml(&tmp.path().join("nonexistent.toml"));
    assert!(result.is_err());
}

#[test]
fn test_load_toml_invalid() {
    let tmp = tempfile::tempdir().unwrap();
    let bad = tmp.path().join("bad.toml");
    std::fs::write(&bad, "this is not [valid toml").unwrap();
    let result = load_toml(&bad);
    assert!(result.is_err());
}

#[test]
fn test_load_toml_valid() {
    let tmp = tempfile::tempdir().unwrap();
    let good = tmp.path().join("good.toml");
    std::fs::write(&good, "[section]\nkey = \"value\"").unwrap();
    let result = load_toml(&good).unwrap();
    let mut expected = HashMap::new();
    expected.insert("section.key".to_string(), "value".to_string());
    assert_eq!(result, expected);
}

// -- TranslationStore ---------------------------------------------------------

#[test]
fn test_store_english() {
    let store = TranslationStore::new("en");
    assert!(!store.all_chat_keys().is_empty());
    assert!(!store.all_cmd_keys().is_empty());
}

#[test]
fn test_store_fallback_missing_key() {
    let store = TranslationStore::new("en");
    let result = store.chat("this.key.does.not.exist", &[]);
    assert_eq!(result, "[MISSING: this.key.does.not.exist]");
}

#[test]
fn test_store_variable_substitution() {
    let store = TranslationStore::new("en");
    let result = store.chat("session.error_body", &[("model", "opus")]);
    assert!(result.contains("opus"));
    assert!(!result.contains("{model}"));
}

#[test]
fn test_store_missing_placeholder_graceful() {
    let store = TranslationStore::new("en");
    let result = store.chat("session.error_body", &[]);
    assert!(result.contains("{model}"));
}

#[test]
fn test_path_traversal_sanitization() {
    let store = TranslationStore::new("../invalid");
    assert_eq!(store.language, "en");

    let store2 = TranslationStore::new("en/../ko");
    assert_eq!(store2.language, "en");
}

#[test]
fn test_empty_string_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let en_dir = root.join("en");
    std::fs::create_dir_all(&en_dir).unwrap();
    std::fs::write(en_dir.join("chat.toml"), "hello = \"hello en\"\nempty_key = \"fallback en\"").unwrap();
    std::fs::write(en_dir.join("cli.toml"), "").unwrap();
    std::fs::write(en_dir.join("commands.toml"), "").unwrap();
    std::fs::write(en_dir.join("wizard.toml"), "").unwrap();

    let de_dir = root.join("de");
    std::fs::create_dir_all(&de_dir).unwrap();
    std::fs::write(de_dir.join("chat.toml"), "hello = \"hallo de\"\nempty_key = \"\"").unwrap();
    std::fs::write(de_dir.join("cli.toml"), "").unwrap();
    std::fs::write(de_dir.join("commands.toml"), "").unwrap();
    std::fs::write(de_dir.join("wizard.toml"), "").unwrap();

    let store = TranslationStore::new_with_root("de", root);
    assert_eq!(store.chat("hello", &[]), "hallo de");
    assert_eq!(store.chat("empty_key", &[]), "fallback en");
}

// -- Public API ----------------------------------------------------------------

#[test]
fn test_init_default() {
    init("en");
    assert_eq!(get_language(), "en");
}

#[test]
fn test_init_english() {
    init("en");
    assert_eq!(get_language(), "en");
}

#[test]
fn test_init_unknown_falls_back_to_english() {
    init("xx_unknown");
    assert_eq!(get_language(), "en");
}

#[test]
fn test_t_returns_string() {
    init("en");
    let result = crate::i18n::t("session.error_header", &[]);
    assert!(result.contains("Session Error"));
}

#[test]
fn test_t_with_kwargs() {
    init("en");
    let result = crate::i18n::t("stop.killed", &[("provider", "Claude")]);
    assert!(result.contains("Claude"));
}

#[test]
fn test_t_rich_returns_string() {
    init("en");
    let result = crate::i18n::t_rich("wizard.common.cancelled", &[]);
    assert!(result.to_lowercase().contains("cancelled"));
}

#[test]
fn test_t_cmd_returns_string() {
    init("en");
    let result = t_cmd("bot.new");
    assert!(!result.is_empty());
}

#[test]
fn test_t_plural_one() {
    init("en");
    let result = crate::i18n::t_plural("tasks.cancelled", 1, &[]);
    assert!(result.contains("1 task."));
}

#[test]
fn test_t_plural_many() {
    init("en");
    let result = crate::i18n::t_plural("tasks.cancelled", 5, &[]);
    assert!(result.contains("5 tasks."));
}

// -- Macro tests ---------------------------------------------------------------

#[test]
fn test_macros() {
    init("en");
    
    let r1 = t!("session.error_header");
    assert!(r1.contains("Session Error"));

    let r2 = t!("stop.killed", provider = "Claude");
    assert!(r2.contains("Claude"));

    let r3 = t_rich!("wizard.common.cancelled");
    assert!(r3.to_lowercase().contains("cancelled"));

    let r4 = t_plural!("tasks.cancelled", 1);
    assert!(r4.contains("1 task."));

    let r5 = t_plural!("tasks.cancelled", 5);
    assert!(r5.contains("5 tasks."));
}

// -- TOML file integrity -------------------------------------------------------

#[test]
fn test_all_chat_keys_resolvable() {
    init("en");
    let store = get_store();
    for key in store.all_chat_keys() {
        let result = store.chat(&key, &[]);
        assert!(!result.contains("[MISSING:"), "Key {:?} is missing", key);
    }
}

#[test]
fn test_all_cmd_keys_resolvable() {
    init("en");
    let store = get_store();
    for key in store.all_cmd_keys() {
        let result = store.cmd(&key);
        assert!(!result.contains("[MISSING:"), "Key {:?} is missing", key);
    }
}

#[test]
fn test_no_empty_values() {
    init("en");
    let store = get_store();
    for key in store.all_chat_keys() {
        let result = store.chat(&key, &[]);
        assert!(!result.trim().is_empty(), "Key {:?} has empty value", key);
    }
}

#[test]
fn test_command_descriptions_short() {
    init("en");
    let store = get_store();
    for key in store.all_cmd_keys() {
        let val = store.cmd(&key);
        assert!(val.len() <= 256, "Command {:?} too long: {} chars", key, val.len());
    }
}

// -- Placeholder consistency ---------------------------------------------------

fn extract_placeholders(text: &str) -> HashSet<String> {
    use regex::Regex;
    let re = Regex::new(r"\{([\w.-]+)\}").unwrap();
    re.captures_iter(text)
        .map(|cap| cap[1].to_string())
        .collect()
}

#[test]
fn test_chat_placeholders_are_valid() {
    init("en");
    let store = get_store();
    for key in store.all_chat_keys() {
        let val = store.chat(&key, &[]);
        let phs = extract_placeholders(&val);
        for ph in phs {
            let is_ident = !ph.is_empty() && ph.chars().next().map_or(false, |c| c.is_ascii_alphabetic() || c == '_')
                && ph.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-');
            assert!(is_ident, "Bad placeholder {{{}}} in {}", ph, key);
        }
    }
}

// -- LANGUAGES dict consistency ------------------------------------------------

#[test]
fn test_languages_has_en() {
    assert!(LANGUAGES.iter().any(|(code, _)| *code == "en"));
}

#[test]
fn test_all_language_dirs_exist() {
    let i18n_dir = find_locales_dir();
    for &(lang_code, _) in LANGUAGES {
        let lang_dir = i18n_dir.join(lang_code);
        assert!(lang_dir.is_dir(), "Language dir missing: {:?}", lang_dir);
    }
}

// -- Completeness --------------------------------------------------------------

#[test]
fn test_key_completeness() {
    for &(lang, _) in LANGUAGES {
        if lang == "en" { continue; }
        let store = TranslationStore::new(lang);
        assert!(store.all_chat_keys().difference(&store.lang_chat_keys()).next().is_none(), "[{}] missing chat keys", lang);
        assert!(store.all_cli_keys().difference(&store.lang_cli_keys()).next().is_none(), "[{}] missing cli keys", lang);
        assert!(store.all_cmd_keys().difference(&store.lang_cmd_keys()).next().is_none(), "[{}] missing cmd keys", lang);
    }
}

#[test]
fn test_placeholder_match() {
    for &(lang, _) in LANGUAGES {
        if lang == "en" { continue; }
        let store = TranslationStore::new(lang);
        for key in &store.all_chat_keys() {
            let en_val = store.en_chat.get(key).cloned().unwrap_or_default();
            let lang_val = store.primary_chat.as_ref().and_then(|m| m.get(key)).cloned().unwrap_or_default();
            if !lang_val.is_empty() {
                assert_eq!(extract_placeholders(&en_val), extract_placeholders(&lang_val), "chat: {}", key);
            }
        }
        for key in &store.all_cli_keys() {
            let en_val = store.en_cli.get(key).cloned().unwrap_or_default();
            let lang_val = store.primary_cli.as_ref().and_then(|m| m.get(key)).cloned().unwrap_or_default();
            if !lang_val.is_empty() {
                assert_eq!(extract_placeholders(&en_val), extract_placeholders(&lang_val), "cli: {}", key);
            }
        }
    }
}

#[test]
fn test_german_loads_without_error() {
    init("de");
    let store = get_store();
    assert_eq!(store.language, "de");
    assert!(!store.lang_chat_keys().is_empty());
    assert!(!store.lang_cli_keys().is_empty());
    assert!(!store.lang_cmd_keys().is_empty());
}
