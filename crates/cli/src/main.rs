//! The `nomi` binary. Command logic lives in other crates; this crate only
//! maps arguments onto them.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("nomi {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("usage: nomi --version");
            ExitCode::from(2)
        }
    }
}
