//! # i18n Checker CLI Utility
//!
//! A command-line wrapper to execute translation completeness checks.

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
