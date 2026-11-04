//! # System Dependency Doctor Auto-Installer (`--fix`)
//!
//! ## Overview
//! Implements `tuner --doctor [--fix]`, automatically downloading user-space companion
//! binaries (`cloudflared`, `agy-vault`), installing missing OS packages (`screen`, `ffmpeg`,
//! `procps`, `nodejs`, `python3-pip`, Chromium CDP shared libraries) when root/passwordless
//! `sudo` is available, and installing Python packages plus Playwright Chromium (`CDP`).
//!
//! ## Search Tags
//! #doctor-fix, #dependency-installer, #cloudflared-installer, #playwright-cdp

use super::doctor::{collect_doctor_checks, find_cloudflared_bin, print_doctor_report, which_bin};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Entrypoint for `tuner --doctor [--fix]`.
pub fn run_doctor_cli(args: &[String]) -> Result<(), String> {
    let fix = args.iter().any(|a| a == "--fix");
    let initial = collect_doctor_checks();
    print_doctor_report(&initial);

    if !fix {
        if initial.missing_count() > 0 {
            println!(
                "\n💡 Found {} missing dependencies. Run `tuner --doctor --fix` to install them automatically.",
                initial.missing_count()
            );
        } else {
            println!("\n🎉 All required Tuner dependencies are installed and verified!");
        }
        return Ok(());
    }

    println!("\n🔧 [tuner --doctor --fix] Starting automated dependency installation & repair...");
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let tuner_bin_dir = PathBuf::from(&home).join(".tuner/bin");
    std::fs::create_dir_all(&tuner_bin_dir).map_err(|e| e.to_string())?;

    if find_cloudflared_bin(&home).is_none() {
        install_cloudflared(&tuner_bin_dir)?;
    }
    if super::find_agy_vault_bin(&home).is_none() {
        install_agy_vault(&home, &tuner_bin_dir);
    }

    install_os_packages_if_needed();
    install_python_and_cdp_if_needed(&home);

    println!("\n🔄 Re-running diagnostics after `--fix`...\n");
    let final_report = collect_doctor_checks();
    print_doctor_report(&final_report);

    if final_report.missing_count() == 0 {
        println!("\n🎉 All required dependencies are now installed and verified!");
        Ok(())
    } else {
        Err(format!(
            "{} dependencies still require attention (see hints above).",
            final_report.missing_count()
        ))
    }
}

fn install_cloudflared(tuner_bin_dir: &Path) -> Result<(), String> {
    let arch = if std::env::consts::ARCH == "aarch64" { "arm64" } else { "amd64" };
    let url = format!(
        "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-{}",
        arch
    );
    let dest = tuner_bin_dir.join("cloudflared");
    let tmp = tuner_bin_dir.join(".cloudflared.tmp");
    println!("📥 [doctor] Downloading cloudflared from {}...", url);
    let status = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(&tmp)
        .arg(&url)
        .status()
        .map_err(|e| format!("Failed to invoke curl for cloudflared: {}", e))?;
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err("Failed to download cloudflared binary".to_string());
    }
    set_executable(&tmp);
    std::fs::rename(&tmp, &dest).map_err(|e| format!("Failed to move cloudflared: {}", e))?;
    println!("✅ [doctor] Installed cloudflared to {}", dest.display());
    Ok(())
}

fn install_agy_vault(home: &str, tuner_bin_dir: &Path) {
    println!("📥 [doctor] Installing companion daemon agy-vault...");
    let tb = tuner_bin_dir.display();
    let script = format!(
        "set -e; TMP=$(mktemp -d); \
         REL=$(curl -fsSL https://api.github.com/repos/imwoo90/agy-vault/releases/latest); \
         TAG=$(echo \"$REL\" | grep -m 1 '\"tag_name\":' | awk -F '\"' '{{print $4}}'); \
         if [ -n \"$TAG\" ]; then \
           curl -fsSL -o \"$TMP/v.tgz\" \"https://github.com/imwoo90/agy-vault/releases/download/$TAG/agy-vault-linux-amd64.tar.gz\"; \
           tar -xzf \"$TMP/v.tgz\" -C \"$TMP\"; \
           cp \"$TMP/agy-vault-linux-amd64/agy-vault\" \"{tb}/agy-vault\"; chmod +x \"{tb}/agy-vault\"; \
           mkdir -p \"{home}/.local/bin\" 2>/dev/null && [ -w \"{home}/.local/bin\" ] && ln -sf \"{tb}/agy-vault\" \"{home}/.local/bin/agy-vault\" || true; \
           \"{tb}/agy-vault\" patch-binary 2>/dev/null || true; \
         fi; rm -rf \"$TMP\""
    );
    if Command::new("bash").arg("-c").arg(&script).status().map(|s| s.success()).unwrap_or(false) {
        println!("✅ [doctor] Installed agy-vault companion daemon.");
    } else {
        eprintln!("⚠️  [doctor] Could not auto-install agy-vault (check network).");
    }
}

fn can_run_privileged_apt() -> Option<&'static str> {
    if Command::new("id").arg("-u").output().map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0").unwrap_or(false) {
        return Some("");
    }
    if Command::new("sudo").args(["-n", "true"]).output().map(|o| o.status.success()).unwrap_or(false) {
        return Some("sudo -n");
    }
    None
}

fn install_os_packages_if_needed() {
    let missing_bin = ["curl", "git", "screen", "pgrep", "ffmpeg", "node"]
        .iter()
        .any(|b| which_bin(b).is_none());
    let pip_ok = Command::new("python3").args(["-m", "pip", "--version"]).output().map(|o| o.status.success()).unwrap_or(false);
    if !missing_bin && pip_ok {
        return;
    }
    let pkgs = "curl git screen procps ffmpeg nodejs python3-pip python3-requests python3-bs4 python3-pil python3-lxml libnss3 libnspr4 libatk1.0-0 libatk-bridge2.0-0 libcups2 libdrm2 libxkbcommon0 libxcomposite1 libxdamage1 libxfixes3 libxrandr2 libgbm1 libpango-1.0-0 libcairo2";
    if let Some(p) = can_run_privileged_apt() {
        println!("📦 [doctor] Installing missing OS & media packages via apt-get...");
        let env_p = if p.is_empty() { "env DEBIAN_FRONTEND=noninteractive" } else { "sudo -n env DEBIAN_FRONTEND=noninteractive" };
        let cmd = format!(
            "{p} apt-get update -qq && {env_p} apt-get install -y -qq {pkgs} && ({env_p} apt-get install -y -qq libasound2t64 || {env_p} apt-get install -y -qq libasound2 || true)"
        );
        let _ = Command::new("bash").arg("-c").arg(&cmd).status();
    } else {
        println!("⚠️  [doctor] Root privileges required for OS packages. Run:\n    sudo apt-get update && sudo apt-get install -y {pkgs} libasound2t64");
    }
}

fn is_dir_writable(dir: &Path) -> bool {
    let _ = std::fs::create_dir_all(dir);
    let probe = dir.join(".tuner_write_probe");
    if std::fs::write(&probe, b"ok").is_ok() {
        let _ = std::fs::remove_file(&probe);
        true
    } else {
        false
    }
}

fn ensure_user_pip(py_path: &Path, home: &str) -> bool {
    if Command::new(py_path).args(["-m", "pip", "--version"]).output().map(|o| o.status.success()).unwrap_or(false) {
        return true;
    }
    println!("📥 [doctor] Bootstrapping user-space pip via get-pip.py...");
    let py = py_path.display();
    let script = if is_dir_writable(&PathBuf::from(home).join(".local/bin")) {
        format!("curl -fsSL https://bootstrap.pypa.io/get-pip.py | {py} - --user --break-system-packages -q")
    } else {
        format!("SITE=$({py} -c 'import site; print(site.getusersitepackages())') && mkdir -p \"$SITE\" && curl -fsSL https://bootstrap.pypa.io/get-pip.py | {py} - --target \"$SITE\" -q")
    };
    let _ = Command::new("bash").arg("-c").arg(&script).status();
    Command::new(py_path).args(["-m", "pip", "--version"]).output().map(|o| o.status.success()).unwrap_or(false)
}

fn install_python_packages(py_path: &Path, home: &str) {
    println!("🐍 [doctor] Ensuring Python core & Vault automation packages...");
    let py_pkgs = ["requests", "beautifulsoup4", "httpx", "pypdf", "pillow", "python-pptx", "openpyxl", "playwright"];
    if is_dir_writable(&PathBuf::from(home).join(".local/bin")) {
        let mut args = vec!["-m", "pip", "install", "--user", "--break-system-packages", "-q"];
        args.extend_from_slice(&py_pkgs);
        if Command::new(py_path).args(&args).status().map(|s| !s.success()).unwrap_or(true) {
            let mut fb = vec!["-m", "pip", "install", "--user", "-q"];
            fb.extend_from_slice(&py_pkgs);
            let _ = Command::new(py_path).args(&fb).status();
        }
    } else if let Ok(out) = Command::new(py_path).args(["-c", "import site; print(site.getusersitepackages())"]).output() {
        let site_dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !site_dir.is_empty() {
            let _ = std::fs::create_dir_all(&site_dir);
            let mut args = vec!["-m", "pip", "install", "--upgrade", "--target", &site_dir, "-q"];
            args.extend_from_slice(&py_pkgs);
            let _ = Command::new(py_path).args(&args).status();
        }
    }
}

fn install_python_and_cdp_if_needed(home: &str) {
    let Some(py_path) = which_bin("python3") else { return };
    if !ensure_user_pip(&py_path, home) {
        return;
    }
    install_python_packages(&py_path, home);
    println!("🌐 [doctor] Ensuring Playwright Chromium headless browser (CDP)...");
    if let Some(p) = can_run_privileged_apt() {
        let cmd = format!("{p} {} -m playwright install-deps chromium", py_path.display());
        let _ = Command::new("bash").arg("-c").arg(cmd.trim()).status();
    }
    let _ = Command::new(&py_path).args(["-m", "playwright", "install", "chromium"]).status();
    fix_home_ownership_if_root(home);
}

fn fix_home_ownership_if_root(home: &str) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if Command::new("id").arg("-u").output().map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0").unwrap_or(false)
            && let Ok(meta) = std::fs::metadata(home)
            && meta.uid() != 0
        {
            let cmd = format!("chown -R {}:{} \"{home}/.tuner/bin\" \"{home}/.local/lib\" \"{home}/.cache\" 2>/dev/null || true", meta.uid(), meta.gid());
            let _ = Command::new("bash").arg("-c").arg(&cmd).status();
        }
    }
}

fn set_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}
