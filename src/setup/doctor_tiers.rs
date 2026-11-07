//! # System Doctor Tier 2 & Tier 3 Diagnostic Checks
//!
//! ## Overview
//! Implements the OS/media utility checks (Tier 2: `curl`, `git`, `screen`, `pgrep`, `ffmpeg`, `node`)
//! and the Python/Playwright CDP headless browser checks (Tier 3) for [`super::doctor`].
//!
//! ## Search Tags
//! #doctor-tiers, #os-tools-check, #python-cdp-check

use super::doctor::{CheckStatus, DoctorCheck, which_bin};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Inspects Tier 1 optional/multi-provider CLI engines (`claude` and `codex`).
pub fn check_tier1_provider_clis(checks: &mut Vec<DoctorCheck>) {
    for (bin, label, prov) in [
        ("claude", "claude (Claude Code CLI)", "claude"),
        ("codex", "codex (OpenAI Codex CLI)", "codex"),
    ] {
        match which_bin(bin) {
            Some(path) => {
                let ver = Command::new(&path)
                    .arg("--version")
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "installed".to_string());
                checks.push(DoctorCheck {
                    tier: "Tier 1: Core Runtime",
                    name: label,
                    status: CheckStatus::Ok,
                    detail: format!("{} (path: {})", ver, path.display()),
                    hint: String::new(),
                });
            }
            None => checks.push(DoctorCheck {
                tier: "Tier 1: Core Runtime",
                name: label,
                status: CheckStatus::Warn,
                detail: format!("Not found in PATH or ~/.local/bin/{}", bin),
                hint: format!("Install `{}` CLI if using the '{}' provider.", bin, prov),
            }),
        }
    }
}

/// Inspects Tier 2 OS and media utilities (`curl`, `git`, `screen`, `pgrep`, `ffmpeg`, `node`).
pub fn check_tier2_os_tools(checks: &mut Vec<DoctorCheck>) {
    let required_bins = [
        ("curl", "HTTP transfer utility"),
        ("git", "Version control"),
        ("screen", "Background session multiplexer (/remote)"),
        ("pgrep", "Process inspection (procps)"),
        ("ffmpeg", "Audio/video media processing (tools/media_tools)"),
    ];
    for (bin, desc) in required_bins {
        match which_bin(bin) {
            Some(p) => checks.push(DoctorCheck {
                tier: "Tier 2: OS & Media Tools",
                name: bin,
                status: CheckStatus::Ok,
                detail: format!("{} ({})", p.display(), desc),
                hint: String::new(),
            }),
            None => checks.push(DoctorCheck {
                tier: "Tier 2: OS & Media Tools",
                name: bin,
                status: CheckStatus::Missing,
                detail: format!("Missing ({})", desc),
                hint: "Run `tuner --doctor --fix` or install via apt-get.".to_string(),
            }),
        }
    }

    match which_bin("node") {
        Some(p) => checks.push(DoctorCheck {
            tier: "Tier 2: OS & Media Tools",
            name: "node (Node.js runtime)",
            status: CheckStatus::Ok,
            detail: p.display().to_string(),
            hint: String::new(),
        }),
        None => checks.push(DoctorCheck {
            tier: "Tier 2: OS & Media Tools",
            name: "node (Node.js runtime)",
            status: CheckStatus::Warn,
            detail: "Missing (optional for JS/MCP tools)".to_string(),
            hint: "Run `tuner --doctor --fix` or install nodejs.".to_string(),
        }),
    }
}

/// Inspects Tier 3 Python interpreter, `pip`, and required core/Vault automation packages.
pub fn check_tier3_python(checks: &mut Vec<DoctorCheck>) -> Option<PathBuf> {
    let Some(py_path) = which_bin("python3") else {
        checks.push(DoctorCheck {
            tier: "Tier 3: Python & CDP Browser",
            name: "python3",
            status: CheckStatus::Missing,
            detail: "Missing Python 3 interpreter".to_string(),
            hint: "Install python3 and python3-pip.".to_string(),
        });
        return None;
    };
    checks.push(DoctorCheck {
        tier: "Tier 3: Python & CDP Browser",
        name: "python3",
        status: CheckStatus::Ok,
        detail: py_path.display().to_string(),
        hint: String::new(),
    });

    let pip_ok = Command::new(&py_path)
        .args(["-m", "pip", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    checks.push(DoctorCheck {
        tier: "Tier 3: Python & CDP Browser",
        name: "python3 -m pip",
        status: if pip_ok {
            CheckStatus::Ok
        } else {
            CheckStatus::Missing
        },
        detail: if pip_ok {
            "Available".to_string()
        } else {
            "Missing pip module".to_string()
        },
        hint: if pip_ok {
            String::new()
        } else {
            "Run `tuner --doctor --fix` to install python3-pip.".to_string()
        },
    });
    check_tier3_python_packages(&py_path, checks);
    Some(py_path)
}

fn check_tier3_python_packages(py_path: &Path, checks: &mut Vec<DoctorCheck>) {
    let py_script = "import importlib.util\n\
mods = ['requests','bs4','httpx','pypdf','PIL','pptx','openpyxl','playwright']\n\
missing = [m for m in mods if importlib.util.find_spec(m) is None]\n\
print(','.join(missing))";
    let missing_mods = Command::new(py_path)
        .args(["-c", py_script])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    if missing_mods.is_empty() {
        checks.push(DoctorCheck {
            tier: "Tier 3: Python & CDP Browser",
            name: "Python Core & Vault Packages",
            status: CheckStatus::Ok,
            detail: "requests, bs4, httpx, pypdf, PIL, pptx, openpyxl, playwright".to_string(),
            hint: String::new(),
        });
    } else {
        checks.push(DoctorCheck {
            tier: "Tier 3: Python & CDP Browser",
            name: "Python Core & Vault Packages",
            status: CheckStatus::Missing,
            detail: format!("Missing modules: {}", missing_mods),
            hint: "Run `tuner --doctor --fix` to install missing Python packages.".to_string(),
        });
    }
}

/// Inspects Tier 3 headless Chromium CDP launch capability via Playwright.
pub fn check_tier3_cdp(py_path: &Path, checks: &mut Vec<DoctorCheck>) {
    let cdp_script = "from playwright.sync_api import sync_playwright\nwith sync_playwright() as p:\n    b = p.chromium.launch(headless=True)\n    b.close()\nprint('CDP_OK')";
    let cdp_out = Command::new(py_path).args(["-c", cdp_script]).output();
    let cdp_ok = cdp_out
        .as_ref()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("CDP_OK"))
        .unwrap_or(false);
    checks.push(DoctorCheck {
        tier: "Tier 3: Python & CDP Browser",
        name: "Playwright Chromium (CDP)",
        status: if cdp_ok {
            CheckStatus::Ok
        } else {
            CheckStatus::Missing
        },
        detail: if cdp_ok {
            "Headless Chromium CDP launch verified".to_string()
        } else {
            "Chromium binary or OS shared libraries missing".to_string()
        },
        hint: if cdp_ok {
            String::new()
        } else {
            "Run `tuner --doctor --fix` to install Playwright Chromium & OS libs.".to_string()
        },
    });
}
