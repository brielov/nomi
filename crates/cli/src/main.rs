//! The `nomi` binary. Command logic lives in other crates; this crate only
//! maps arguments onto them.

use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// One toolchain for Elm: compiler, formatter, language server, test runner
/// and package manager.
#[derive(Debug, Parser)]
#[command(name = "nomi", version, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

/// Subcommands are added by the plan steps that implement them. Each one
/// declares the Elm names it answers to as aliases, e.g.
/// `#[command(visible_alias = "make")] Build`.
#[derive(Debug, Subcommand)]
enum Command {}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        None => ExitCode::SUCCESS,
    }
}
