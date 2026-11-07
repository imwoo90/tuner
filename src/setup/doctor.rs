//! # System Dependency Doctor Diagnostics
//!
//! ## Overview
//! Inspects all runtime dependencies required by Tuner across three tiers:
//! 1. Core runtime & companion binaries (`agy`, `agy-vault`, `cloudflared`, `_home_defaults`).
//! 2. OS & media utilities (`curl`, `git`, `screen`, `pgrep`, `ffmpeg`, `node`).
//! 3. Python runtime, packages (`requests`, `bs4`, `httpx`, `pypdf`, `PIL`, `pptx`, `openpyxl`, `playwright`), and headless Chromium CDP readiness.
//!
//! ## Search Tags
//! #system-doctor, #dependency-checks, #cloudflared-check, #cdp-browser-check

use super::doctor_tiers::{
    check_tier1_provider_clis, check_tier2_os_tools, check_tier3_cdp, check_tier3_python,
};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Status of an individual dependency diagnostic check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckStatus {
    /// Dependency is present and healthy.
    Ok,
    /// Optional dependency or non-fatal configuration warning.
    Warn,
    /// Required dependency is missing or broken.
    Missing,
}

/// Result of a single dependency diagnostic item.
#[derive(Clone, Debug)]
pub struct DoctorCheck {
    /// Tier category label.
    pub tier: &'static str,
    /// Dependency identifier.
    pub name: &'static str,
    /// Diagnostic status.
    pub status: CheckStatus,
    /// Version or path detail string.
    pub detail: String,
    /// Actionable remediation hint when not `Ok`.
    pub hint: String,
}

/// Aggregated report returned by [`collect_doctor_checks`].
#[derive(Clone, Debug)]
pub struct DoctorReport {
    /// Ordered list of diagnostic check items.
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    /// Returns the number of checks with [`CheckStatus::Missing`].
    pub fn missing_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| c.status == CheckStatus::Missing)
            .count()
    }

    /// Returns the number of checks with [`CheckStatus::Warn`].
    pub fn warn_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|c| c.status == CheckStatus::Warn)
            .count()
    }
}

/// Locates `cloudflared` in user/tuner/system binary directories.
pub fn find_cloudflared_bin(home: &str) -> Option<PathBuf> {
    let mut candidates = vec![
        PathBuf::from(home).join(".tuner/bin/cloudflared"),
        PathBuf::from(home).join(".local/bin/cloudflared"),
    ];
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        candidates.push(parent.join("cloudflared"));
    }
    candidates.push(PathBuf::from("/usr/local/bin/cloudflared"));
    candidates.push(PathBuf::from("/usr/bin/cloudflared"));
    candidates.into_iter().find(|p| p.is_file())
}

/// Locates the `agy` CLI binary in standard Antigravity or PATH locations.
pub fn find_agy_bin(home: &str) -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(home).join(".gemini/antigravity-cli/bin/agy"),
        PathBuf::from(home).join(".local/bin/agy"),
        PathBuf::from("/usr/local/bin/agy"),
        PathBuf::from("/usr/bin/agy"),
    ];
    if let Some(found) = candidates.into_iter().find(|p| p.is_file()) {
        return Some(found);
    }
    which_bin("agy")
}

/// Resolves a binary name using `which` or standard user paths.
pub fn which_bin(name: &str) -> Option<PathBuf> {
    if let Ok(out) = Command::new("which").arg(name).output()
        && out.status.success()
    {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            return Some(PathBuf::from(s));
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    for sub in [".tuner/bin", ".local/bin", ".gemini/antigravity-cli/bin"] {
        let p = PathBuf::from(&home).join(sub).join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn check_tier1_agy(home: &str, checks: &mut Vec<DoctorCheck>) {
    let verified_agy = env!("TUNER_VERIFIED_AGY_VERSION");
    match find_agy_bin(home) {
        Some(path) => {
            let ver = Command::new(&path)
                .arg("--version")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            checks.push(DoctorCheck {
                tier: "Tier 1: Core Runtime",
                name: "agy (Antigravity CLI)",
                status: CheckStatus::Ok,
                detail: format!("{} (verified: {}, path: {})", ver, verified_agy, path.display()),
                hint: String::new(),
            });
        }
        None => checks.push(DoctorCheck {
            tier: "Tier 1: Core Runtime",
            name: "agy (Antigravity CLI)",
            status: CheckStatus::Warn,
            detail: "Not found in ~/.gemini/antigravity-cli/bin/agy".to_string(),
            hint: "Install Antigravity CLI if using the 'antigravity' provider.".to_string(),
        }),
    }

    let oauth_tok = PathBuf::from(home).join(".gemini/antigravity-cli/antigravity-oauth-token");
    if oauth_tok.is_file() {
        checks.push(DoctorCheck {
            tier: "Tier 1: Core Runtime",
            name: "agy OAuth Token",
            status: CheckStatus::Ok,
            detail: format!("Signed in ({})", oauth_tok.display()),
            hint: String::new(),
        });
    } else {
        checks.push(DoctorCheck {
            tier: "Tier 1: Core Runtime",
            name: "agy OAuth Token",
            status: CheckStatus::Warn,
            detail: "Not signed in".to_string(),
            hint: "Run `agy` interactively to complete Google OAuth sign-in.".to_string(),
        });
    }
}

fn push_tier1(
    checks: &mut Vec<DoctorCheck>,
    name: &'static str,
    status: CheckStatus,
    detail: String,
    hint: &str,
) {
    checks.push(DoctorCheck {
        tier: "Tier 1: Core Runtime",
        name,
        status,
        detail,
        hint: hint.to_string(),
    });
}

fn check_tier1_companions(home: &str, checks: &mut Vec<DoctorCheck>) {
    match super::find_agy_vault_bin(home) {
        Some(path) => push_tier1(
            checks,
            "agy-vault (Session Daemon)",
            CheckStatus::Ok,
            path.display().to_string(),
            "",
        ),
        None => push_tier1(
            checks,
            "agy-vault (Session Daemon)",
            CheckStatus::Missing,
            "Missing companion binary".to_string(),
            "Run `tuner --doctor --fix` to auto-download agy-vault.",
        ),
    }

    match find_cloudflared_bin(home) {
        Some(path) => push_tier1(
            checks,
            "cloudflared (Web Tunnel)",
            CheckStatus::Ok,
            path.display().to_string(),
            "",
        ),
        None => push_tier1(
            checks,
            "cloudflared (Web Tunnel)",
            CheckStatus::Missing,
            "Missing (required for /vault & /review external URLs)".to_string(),
            "Run `tuner --doctor --fix` to auto-download cloudflared into ~/.tuner/bin.",
        ),
    }

    let defaults_ok = PathBuf::from(home).join(".tuner/bin/_home_defaults").is_dir()
        || Path::new("_home_defaults").is_dir();
    push_tier1(
        checks,
        "_home_defaults (Templates)",
        if defaults_ok { CheckStatus::Ok } else { CheckStatus::Warn },
        if defaults_ok {
            "Present".to_string()
        } else {
            "Not in ~/.tuner/bin/_home_defaults".to_string()
        },
        if defaults_ok {
            ""
        } else {
            "Re-run install.sh or copy _home_defaults to ~/.tuner/bin."
        },
    );
}

/// Runs all dependency diagnostic checks and returns a [`DoctorReport`].
pub fn collect_doctor_checks() -> DoctorReport {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let mut checks = Vec::new();
    check_tier1_agy(&home, &mut checks);
    check_tier1_provider_clis(&mut checks);
    check_tier1_companions(&home, &mut checks);
    check_tier2_os_tools(&mut checks);
    if let Some(py_path) = check_tier3_python(&mut checks) {
        check_tier3_cdp(&py_path, &mut checks);
    }
    DoctorReport { checks }
}

/// Prints a formatted human-readable [`DoctorReport`] to stdout.
pub fn print_doctor_report(report: &DoctorReport) {
    println!("🩺 [tuner --doctor] System Dependency & Runtime Diagnostics");
    println!("========================================================================");
    let mut current_tier = "";
    for item in &report.checks {
        if item.tier != current_tier {
            current_tier = item.tier;
            println!("\n[{}]", current_tier);
        }
        let badge = match item.status {
            CheckStatus::Ok => "✅ [OK]     ",
            CheckStatus::Warn => "⚠️  [WARN]   ",
            CheckStatus::Missing => "❌ [MISSING]",
        };
        println!("  {} {:<30} — {}", badge, item.name, item.detail);
        if !item.hint.is_empty() {
            println!("               💡 {}", item.hint);
        }
    }
    println!("\n========================================================================");
    println!(
        "Summary: {} OK, {} warnings, {} missing.",
        report
            .checks
            .iter()
            .filter(|c| c.status == CheckStatus::Ok)
            .count(),
        report.warn_count(),
        report.missing_count()
    );
}
