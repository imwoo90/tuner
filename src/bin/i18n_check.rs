//! # Internationalization Syntax Validator Tool
//!
//! ## Overview
//! A developer CLI tool designed to run in git pre-commit hooks or CI. Audits i18n TOML files
//! to identify missing keys, placeholder discrepancies, or broken formatting.
//!
//! ## Submodules
//! - [`i18n`] (`../i18n/mod.rs`): Shared internationalization registry and checker logic.
//!
//! ## Search Tags
//! #i18n-check, #cli-validator, #translation-audit

#[path = "../i18n/mod.rs"]
pub mod i18n;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let slice = if args.is_empty() {
        &[][..]
    } else {
        &args[1..]
    };
    let exit_code = i18n::check::run_checker(slice);
    std::process::exit(exit_code);
}
