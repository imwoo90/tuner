//! # i18n Completeness Checker
//!
//! Compares translated locale files against the English source of truth to identify
//! missing keys, extra keys, and mismatched formatting placeholders.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// The list of translation domains (TOML files) verified by the checker.
pub const DOMAINS: &[&str] = &["chat", "cli", "commands", "wizard"];

/// Represents a mismatch in formatting placeholders for a specific key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceholderMismatch {
    pub key: String,
    pub en_only: BTreeSet<String>,
    pub locale_only: BTreeSet<String>,
}

/// Completeness report for a single domain.
#[derive(Debug, Clone, Default)]
pub struct DomainReport {
    pub missing: Vec<String>,
    pub extra: Vec<String>,
    pub placeholder_mismatches: Vec<PlaceholderMismatch>,
    pub empty: Vec<String>,
}

impl DomainReport {
    /// Return true if the domain has no missing, extra, or mismatched keys, and no empty values.
    pub fn clean(&self) -> bool {
        self.missing.is_empty() && self.extra.is_empty() && self.placeholder_mismatches.is_empty() && self.empty.is_empty()
    }

    /// Return the total number of issues identified in this domain.
    pub fn total_issues(&self) -> usize {
        self.missing.len() + self.extra.len() + self.placeholder_mismatches.len() + self.empty.len()
    }
}

/// Completeness report for a single locale containing multiple domains.
#[derive(Debug, Clone)]
pub struct LocaleReport {
    pub locale: String,
    pub domains: HashMap<String, DomainReport>,
}

impl LocaleReport {
    /// Return true if all domains in the locale are fully synced and clean.
    pub fn clean(&self) -> bool {
        self.domains.values().all(|d| d.clean())
    }

    /// Return the total number of issues identified across all domains in the locale.
    pub fn total_issues(&self) -> usize {
        self.domains.values().map(|d| d.total_issues()).sum()
    }
}

/// Top-level report containing completeness results for all tested locales.
#[derive(Debug, Clone)]
pub struct Report {
    pub root: PathBuf,
    pub locales: Vec<LocaleReport>,
}

impl Report {
    /// Return true if all locales in the report are fully synced and clean.
    pub fn clean(&self) -> bool {
        self.locales.iter().all(|l| l.clean())
    }
}

/// Extract all placeholders (e.g. `{count}`) from a translation string.
pub fn placeholders(text: &str) -> BTreeSet<String> {
    use regex::Regex;
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\{([\w.-]+)\}").unwrap());
    re.captures_iter(text)
        .map(|cap| cap[1].to_string())
        .collect()
}

/// Load a specific domain (TOML file) for a given locale.
pub fn load_domain(root: &Path, locale: &str, domain: &str) -> HashMap<String, String> {
    super::load_toml(&root.join(locale).join(format!("{}.toml", domain))).unwrap_or_default()
}

/// Compare a translated domain against the English source of truth.
pub fn compare_domain(en: &HashMap<String, String>, tr: &HashMap<String, String>) -> DomainReport {
    let en_keys: BTreeSet<&String> = en.keys().collect();
    let tr_keys: BTreeSet<&String> = tr.keys().collect();

    let missing: Vec<String> = en_keys.difference(&tr_keys)
        .map(|k| k.to_string())
        .collect();

    let extra: Vec<String> = tr_keys.difference(&en_keys)
        .map(|k| k.to_string())
        .collect();

    let mut empty = Vec::new();
    let mut placeholder_mismatches = Vec::new();
    let common: BTreeSet<&String> = en_keys.intersection(&tr_keys).cloned().collect();
    for key in common {
        let en_val = en.get(key).unwrap();
        let tr_val = tr.get(key).unwrap();

        if tr_val.is_empty() {
            empty.push(key.clone());
            continue;
        }

        let en_ph = placeholders(en_val);
        let tr_ph = placeholders(tr_val);

        if en_ph != tr_ph {
            let en_only: BTreeSet<String> = en_ph.difference(&tr_ph).cloned().collect();
            let locale_only: BTreeSet<String> = tr_ph.difference(&en_ph).cloned().collect();
            placeholder_mismatches.push(PlaceholderMismatch {
                key: key.clone(),
                en_only,
                locale_only,
            });
        }
    }

    DomainReport {
        missing,
        extra,
        placeholder_mismatches,
        empty,
    }
}

/// Build a completeness report for specified locales or all supported locales.
///
/// # Examples
/// ```no_run
/// use std::path::Path;
/// let report = tuner::i18n::check::build_report(Path::new("src/i18n/locales"), None);
/// assert!(report.is_ok());
/// ```
pub fn build_report(root: &Path, locales: Option<Vec<String>>) -> anyhow::Result<Report> {
    if !root.join("en").is_dir() {
        anyhow::bail!("English source-of-truth locale not found under {:?}", root);
    }

    let target_locales = match locales {
        Some(list) => list,
        None => super::LANGUAGES.iter()
            .map(|(code, _)| code.to_string())
            .filter(|code| code != "en")
            .collect(),
    };

    let mut en_data = HashMap::new();
    for domain in DOMAINS {
        en_data.insert(domain.to_string(), load_domain(root, "en", domain));
    }

    let mut locales_reports = Vec::new();
    for locale in target_locales {
        if locale == "en" {
            continue;
        }
        let mut domains_reports = HashMap::new();
        for domain in DOMAINS {
            let tr = load_domain(root, &locale, domain);
            let en = en_data.get(*domain).unwrap();
            domains_reports.insert(domain.to_string(), compare_domain(en, &tr));
        }
        locales_reports.push(LocaleReport {
            locale,
            domains: domains_reports,
        });
    }

    Ok(Report {
        root: root.to_path_buf(),
        locales: locales_reports,
    })
}

fn format_matrix(report: &Report) -> Vec<String> {
    let mut lines = vec![
        "## Summary matrix (missing + extra + placeholder mismatches)".to_string(),
        "".to_string(),
        "| locale | chat | cli | commands | wizard | total |".to_string(),
        "|---|---|---|---|---|---|".to_string(),
    ];
    for loc in &report.locales {
        let chat_issues = loc.domains.get("chat").map_or(0, |d| d.total_issues());
        let cli_issues = loc.domains.get("cli").map_or(0, |d| d.total_issues());
        let cmd_issues = loc.domains.get("commands").map_or(0, |d| d.total_issues());
        let wiz_issues = loc.domains.get("wizard").map_or(0, |d| d.total_issues());
        lines.push(format!(
            "| {} | {} | {} | {} | {} | {} |",
            loc.locale, chat_issues, cli_issues, cmd_issues, wiz_issues, loc.total_issues()
        ));
    }
    lines.push("".to_string());
    lines
}

fn format_domain_detail(domain: &str, locale: &str, d: &DomainReport) -> Vec<String> {
    let mut lines = vec![
        format!("**{}.toml** - {} issue(s)", domain, d.total_issues()),
        "".to_string(),
    ];
    if !d.missing.is_empty() {
        lines.push(format!("- missing ({}):", d.missing.len()));
        for k in &d.missing {
            lines.push(format!("    - `{}`", k));
        }
    }
    if !d.extra.is_empty() {
        lines.push(format!("- extra / stale ({}):", d.extra.len()));
        for k in &d.extra {
            lines.push(format!("    - `{}`", k));
        }
    }
    if !d.empty.is_empty() {
        lines.push(format!("- empty translation ({}):", d.empty.len()));
        for k in &d.empty {
            lines.push(format!("    - `{}`", k));
        }
    }
    if !d.placeholder_mismatches.is_empty() {
        lines.push(format!("- placeholder mismatch ({}):", d.placeholder_mismatches.len()));
        for pm in &d.placeholder_mismatches {
            let only_en = if pm.en_only.is_empty() {
                "-".to_string()
            } else {
                pm.en_only.iter().map(|p| format!("{{{}}}", p)).collect::<Vec<_>>().join(", ")
            };
            let only_loc = if pm.locale_only.is_empty() {
                "-".to_string()
            } else {
                pm.locale_only.iter().map(|p| format!("{{{}}}", p)).collect::<Vec<_>>().join(", ")
            };
            lines.push(format!(
                "    - `{}`: en-only {} | {}-only {}",
                pm.key, only_en, locale, only_loc
            ));
        }
    }
    lines.push("".to_string());
    lines
}

fn format_locale_detail(loc: &LocaleReport) -> Vec<String> {
    let mut lines = vec![
        format!("### {}", loc.locale),
        "".to_string(),
    ];
    for domain in DOMAINS {
        if let Some(d) = loc.domains.get(*domain) {
            if d.clean() {
                continue;
            }
            lines.extend(format_domain_detail(domain, &loc.locale, d));
        }
    }
    lines
}

/// Format a Report into a markdown string summary.
pub fn format_report(report: &Report) -> String {
    let mut lines = vec![
        "# i18n completeness report".to_string(),
        "".to_string(),
        format!("Source of truth: `en`  -  Root: `{}`", report.root.to_string_lossy()),
        "".to_string(),
    ];

    lines.extend(format_matrix(report));

    if report.clean() {
        lines.push("All locales fully synced with en. No gaps.".to_string());
        lines.push("".to_string());
        return lines.join("\n");
    }

    lines.push("## Details".to_string());
    lines.push("".to_string());
    for loc in &report.locales {
        if loc.clean() {
            continue;
        }
        lines.extend(format_locale_detail(loc));
    }

    lines.join("\n")
}

/// Main execution function for the i18n-check CLI entrypoint.
pub fn run_checker(argv: &[String]) -> i32 {
    let mut root_path = None;
    let mut quiet = false;

    let mut i = 0;
    while i < argv.len() {
        if argv[i] == "--root" {
            if i + 1 < argv.len() {
                root_path = Some(argv[i + 1].clone());
                i += 2;
            } else {
                eprintln!("error: --root requires a value");
                return 2;
            }
        } else if argv[i] == "--quiet" {
            quiet = true;
            i += 1;
        } else {
            eprintln!("error: unknown argument {}", argv[i]);
            return 2;
        }
    }

    let root = match root_path {
        Some(p) => PathBuf::from(p),
        None => super::find_locales_dir(),
    };

    match build_report(&root, None) {
        Ok(report) => {
            if !quiet {
                println!("{}", format_report(&report));
            }
            if report.clean() {
                0
            } else {
                1
            }
        }
        Err(e) => {
            eprintln!("error: {}", e);
            2
        }
    }
}
