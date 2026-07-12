use crate::i18n::{LANGUAGES, find_locales_dir};
use crate::i18n::check::{
    placeholders, compare_domain, build_report, format_report, run_checker,
    DomainReport, LocaleReport, Report
};
use std::collections::{HashMap, HashSet};

#[test]
fn test_ph_none() {
    assert!(placeholders("plain text").is_empty());
}

#[test]
fn test_ph_multiple() {
    let phs = placeholders("hello {name}, you have {count} msgs");
    let mut exp = HashSet::new();
    exp.insert("name".to_string());
    exp.insert("count".to_string());
    assert_eq!(phs, exp.into_iter().collect());
}

#[test]
fn test_ph_dup() {
    let phs = placeholders("{x} {x} {y}");
    let mut exp = HashSet::new();
    exp.insert("x".to_string());
    exp.insert("y".to_string());
    assert_eq!(phs, exp.into_iter().collect());
}

#[test]
fn test_compare_identical() {
    let mut en = HashMap::new();
    en.insert("a".to_string(), "hi".to_string());
    en.insert("b".to_string(), "hello {name}".to_string());

    let mut tr = HashMap::new();
    tr.insert("a".to_string(), "hola".to_string());
    tr.insert("b".to_string(), "hola {name}".to_string());

    let r = compare_domain(&en, &tr);
    assert!(r.clean());
    assert_eq!(r.total_issues(), 0);
}

#[test]
fn test_compare_missing() {
    let mut en = HashMap::new();
    en.insert("a".to_string(), "hi".to_string());
    en.insert("b".to_string(), "bye".to_string());

    let mut tr = HashMap::new();
    tr.insert("a".to_string(), "hola".to_string());

    let r = compare_domain(&en, &tr);
    assert_eq!(r.missing, vec!["b".to_string()]);
    assert!(r.extra.is_empty());
    assert!(r.placeholder_mismatches.is_empty());
}

#[test]
fn test_compare_extra() {
    let mut en = HashMap::new();
    en.insert("a".to_string(), "hi".to_string());

    let mut tr = HashMap::new();
    tr.insert("a".to_string(), "hola".to_string());
    tr.insert("stale".to_string(), "garbage".to_string());

    let r = compare_domain(&en, &tr);
    assert!(r.missing.is_empty());
    assert_eq!(r.extra, vec!["stale".to_string()]);
}

#[test]
fn test_compare_mismatch() {
    let mut en = HashMap::new();
    en.insert("a".to_string(), "hi {name}".to_string());

    let mut tr = HashMap::new();
    tr.insert("a".to_string(), "hola {nombre}".to_string());

    let r = compare_domain(&en, &tr);
    assert_eq!(r.placeholder_mismatches.len(), 1);
    let pm = &r.placeholder_mismatches[0];
    assert_eq!(pm.key, "a");
    assert_eq!(pm.en_only.iter().cloned().collect::<Vec<_>>(), vec!["name".to_string()]);
    assert_eq!(pm.locale_only.iter().cloned().collect::<Vec<_>>(), vec!["nombre".to_string()]);
}

#[test]
fn test_compare_empty() {
    let mut en = HashMap::new();
    en.insert("a".to_string(), "hi".to_string());
    en.insert("b".to_string(), "bye".to_string());

    let mut tr = HashMap::new();
    tr.insert("a".to_string(), "hola".to_string());
    tr.insert("b".to_string(), "".to_string());

    let r = compare_domain(&en, &tr);
    assert_eq!(r.empty, vec!["b".to_string()]);
    assert!(r.missing.is_empty());
    assert!(r.extra.is_empty());
    assert!(r.placeholder_mismatches.is_empty());
    assert!(!r.clean());
    assert_eq!(r.total_issues(), 1);
}

#[test]
fn test_reorder_ph() {
    let mut en = HashMap::new();
    en.insert("a".to_string(), "{x} then {y}".to_string());

    let mut tr = HashMap::new();
    tr.insert("a".to_string(), "{y} primero, luego {x}".to_string());

    let r = compare_domain(&en, &tr);
    assert!(r.clean());
}

#[test]
fn test_live_tree_clean() {
    let root = find_locales_dir();
    let r = build_report(&root, None).unwrap();
    assert!(r.clean(), "{}", format_report(&r));
}

#[test]
fn test_report_locales() {
    let root = find_locales_dir();
    let r = build_report(&root, None).unwrap();
    let exp: HashSet<String> = LANGUAGES.iter()
        .map(|(code, _)| code.to_string())
        .filter(|code| code != "en")
        .collect();
    let act: HashSet<String> = r.locales.iter().map(|l| l.locale.clone()).collect();
    assert_eq!(act, exp);
}

#[test]
fn test_missing_en() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(build_report(tmp.path(), None).is_err());
}

fn write_locale(root: &std::path::Path, locale: &str, files: &[(&str, &str)]) {
    let d = root.join(locale);
    std::fs::create_dir_all(&d).unwrap();
    for (name, content) in files {
        std::fs::write(d.join(format!("{}.toml", name)), content).unwrap();
    }
}

#[test]
fn test_synthetic_gaps() {
    let tmp = tempfile::tempdir().unwrap();
    write_locale(tmp.path(), "en", &[
        ("chat", "g = \"H {name}\"\nf = \"B\""),
        ("cli", "s = \"S\""),
        ("commands", "h = \"H\""),
        ("wizard", "w = \"W {n}\""),
    ]);
    write_locale(tmp.path(), "xx", &[
        ("chat", "g = \"H {nombre}\""),
        ("cli", "s = \"S\"\nstale_key = \"garbage\""),
        ("commands", "h = \"H\""),
        ("wizard", ""),
    ]);
    let r = build_report(tmp.path(), Some(vec!["xx".to_string()])).unwrap();
    assert!(!r.clean());
    let loc = &r.locales[0];
    assert_eq!(loc.locale, "xx");

    let chat = loc.domains.get("chat").unwrap();
    assert_eq!(chat.missing, vec!["f".to_string()]);
    assert_eq!(chat.placeholder_mismatches[0].key, "g");

    let cli = loc.domains.get("cli").unwrap();
    assert_eq!(cli.extra, vec!["stale_key".to_string()]);

    let wizard = loc.domains.get("wizard").unwrap();
    assert_eq!(wizard.missing, vec!["w".to_string()]);
}

#[test]
fn test_locale_filter() {
    let tmp = tempfile::tempdir().unwrap();
    write_locale(tmp.path(), "en", &[("chat", "a = \"x\"")]);
    write_locale(tmp.path(), "only", &[("chat", "a = \"x\"")]);
    let r = build_report(tmp.path(), Some(vec!["only".to_string()])).unwrap();
    let act: Vec<String> = r.locales.iter().map(|l| l.locale.clone()).collect();
    assert_eq!(act, vec!["only".to_string()]);
}

#[test]
fn test_skips_en() {
    let tmp = tempfile::tempdir().unwrap();
    write_locale(tmp.path(), "en", &[("chat", "a = \"x\"")]);
    let r = build_report(tmp.path(), Some(vec!["en".to_string()])).unwrap();
    assert!(r.locales.is_empty());
}

#[test]
fn test_format_clean() {
    let tmp = tempfile::tempdir().unwrap();
    write_locale(tmp.path(), "en", &[("chat", "a = \"x\"")]);
    write_locale(tmp.path(), "de", &[("chat", "a = \"y\"")]);
    let r = build_report(tmp.path(), Some(vec!["de".to_string()])).unwrap();
    let output = format_report(&r);
    assert!(output.contains("All locales fully synced with en"));
    assert!(output.contains("| de |"));
}

#[test]
fn test_format_details() {
    let tmp = tempfile::tempdir().unwrap();
    write_locale(tmp.path(), "en", &[("chat", "a = \"x\"\nb = \"y {foo}\"")]);
    write_locale(tmp.path(), "de", &[("chat", "a = \"x\"\nb = \"y {bar}\"\nstale = \"z\"")]);
    let r = build_report(tmp.path(), Some(vec!["de".to_string()])).unwrap();
    let output = format_report(&r);
    assert!(output.contains("### de"));
    assert!(output.contains("**chat.toml**"));
    assert!(output.contains("stale"));
    assert!(output.contains("`b`"));
    assert!(output.contains("{foo}"));
    assert!(output.contains("{bar}"));
}

#[test]
fn test_run_checker_clean() {
    let root = find_locales_dir();
    let rc = run_checker(&["--root".to_string(), root.to_string_lossy().to_string()]);
    assert_eq!(rc, 0);
}

#[test]
fn test_run_checker_gaps() {
    let tmp = tempfile::tempdir().unwrap();
    write_locale(tmp.path(), "en", &[("chat", "a = \"x\"\nb = \"y\"")]);
    write_locale(tmp.path(), "de", &[("chat", "a = \"x\"")]);
    let rc = run_checker(&[
        "--root".to_string(),
        tmp.path().to_string_lossy().to_string(),
    ]);
    assert_eq!(rc, 1);
}

#[test]
fn test_run_checker_quiet() {
    let tmp = tempfile::tempdir().unwrap();
    write_locale(tmp.path(), "en", &[("chat", "a = \"x\"")]);
    write_locale(tmp.path(), "de", &[("chat", "a = \"x\"")]);
    let rc = run_checker(&[
        "--root".to_string(),
        tmp.path().to_string_lossy().to_string(),
        "--quiet".to_string(),
    ]);
    assert_eq!(rc, 1);
}

#[test]
fn test_run_checker_bad() {
    let tmp = tempfile::tempdir().unwrap();
    let empty = tmp.path().join("no_en_here");
    std::fs::create_dir_all(&empty).unwrap();
    let rc = run_checker(&[
        "--root".to_string(),
        empty.to_string_lossy().to_string(),
    ]);
    assert_eq!(rc, 2);
}

#[test]
fn test_report_clean_counts() {
    let mut d = DomainReport::default();
    assert!(d.clean());
    assert_eq!(d.total_issues(), 0);
    d.missing.push("k".to_string());
    assert!(!d.clean());
    assert_eq!(d.total_issues(), 1);
}

#[test]
fn test_locale_report_aggregates() {
    let mut loc = LocaleReport {
        locale: "xx".to_string(),
        domains: HashMap::new(),
    };
    loc.domains.insert("chat".to_string(), DomainReport {
        missing: vec!["a".to_string(), "b".to_string()],
        extra: vec![],
        placeholder_mismatches: vec![],
        empty: vec![],
    });
    loc.domains.insert("cli".to_string(), DomainReport::default());
    assert!(!loc.clean());
    assert_eq!(loc.total_issues(), 2);
}

#[test]
fn test_report_clean_requires_all() {
    let mut r = Report {
        root: std::path::PathBuf::from("/tmp"),
        locales: vec![
            LocaleReport {
                locale: "xx".to_string(),
                domains: HashMap::new(),
            }
        ],
    };
    r.locales[0].domains.insert("chat".to_string(), DomainReport::default());
    assert!(r.clean());
    r.locales[0].domains.get_mut("chat").unwrap().missing.push("k".to_string());
    assert!(!r.clean());
}
