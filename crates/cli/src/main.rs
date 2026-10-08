//! The `nomi` binary. Command logic lives in other crates; this crate only
//! maps arguments onto them.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use nomi_engine::Vfs;
use nomi_engine::format::{self, Mode, Outcome};

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
enum Command {
    /// Format Elm modules as elm-format 0.8.8 does.
    #[command(visible_alias = "fmt")]
    Format {
        /// Files, or directories to search for `.elm` files.
        #[arg(default_value = ".", conflicts_with = "stdin")]
        paths: Vec<PathBuf>,
        /// Change no files; list those that aren't formatted, and fail if
        /// there are any.
        #[arg(long)]
        check: bool,
        /// Format standard input to standard output.
        #[arg(long)]
        stdin: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let succeeded = match cli.command {
        None => true,
        Some(Command::Format {
            paths,
            check,
            stdin,
        }) => {
            let mode = if check { Mode::Check } else { Mode::Write };
            if stdin {
                format_stdin(mode)
            } else {
                format_paths(&paths, mode)
            }
        }
    };
    if succeeded {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn format_paths(paths: &[PathBuf], mode: Mode) -> bool {
    let report = format::format_paths(&Vfs::new(), paths, mode);
    for (root, error) in &report.roots {
        eprintln!("error: {}: {error}", root.display());
    }
    for (file, outcome) in &report.files {
        match outcome {
            Outcome::Unchanged => {}
            Outcome::Formatted if mode == Mode::Check => println!("{}", file.display()),
            Outcome::Formatted => {}
            Outcome::Invalid(at) => {
                eprintln!(
                    "error: {}:{}:{}: syntax error",
                    file.display(),
                    at.row,
                    at.column
                )
            }
            Outcome::Failed(e) => eprintln!("error: {}: {e}", file.display()),
        }
    }
    report.succeeded(mode)
}

fn format_stdin(mode: Mode) -> bool {
    let mut text = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut text) {
        eprintln!("error: <stdin>: {e}");
        return false;
    }
    match format::format_text(&text) {
        Ok(formatted) if mode == Mode::Check => {
            let unchanged = formatted == text;
            if !unchanged {
                println!("<stdin>");
            }
            unchanged
        }
        Ok(formatted) => io::stdout().write_all(formatted.as_bytes()).is_ok(),
        Err(at) => {
            eprintln!("error: <stdin>:{}:{}: syntax error", at.row, at.column);
            false
        }
    }
}
