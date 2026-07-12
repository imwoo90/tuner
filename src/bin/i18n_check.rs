#[path = "../i18n/mod.rs"]
pub mod i18n;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let exit_code = i18n::check::run_checker(&args[1..]);
    std::process::exit(exit_code);
}
