//! Build script for rust-agent-native-template.
//!
//! Integrates the Agent-Native linter and provides a clean, welcoming canvas for
//! user-defined pre-build logic (code generation, C FFI, assets, etc.).

#[path = "build_linter.rs"]
mod build_linter;

fn inject_verified_agy_version() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    let mut verified = "unknown".to_string();
    if let Ok(cargo_toml) = std::fs::read_to_string("Cargo.toml") {
        for line in cargo_toml.lines() {
            if let Some(rest) = line.trim().strip_prefix("verified_version")
                && let Some((_, val)) = rest.split_once('=')
            {
                verified = val.trim().trim_matches('"').to_string();
                break;
            }
        }
    }
    println!("cargo:rustc-env=TUNER_VERIFIED_AGY_VERSION={}", verified);
}

fn main() {
    // 1. Enforce Agent-Native architectural constraints (AGENTS.md Rules 1-4).
    // Thresholds can be customized in `.agent-lint.toml`.
    build_linter::run_linter();

    // 2. Inject verified Antigravity CLI version from Cargo.toml metadata.
    inject_verified_agy_version();
}
