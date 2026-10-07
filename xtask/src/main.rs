//! Repository automation, run as `cargo xtask <command>`.

mod checks;

use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "cargo xtask")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Tier 1: format check, clippy, tests and architecture checks.
    Ci,
    /// Only the architecture checks.
    Check,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Ci => ci(),
        Command::Check => architecture(&root()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace root")
        .to_path_buf()
}

fn ci() -> Result<()> {
    let root = root();
    cargo(&root, &["fmt", "--all", "--check"])?;
    cargo(
        &root,
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    cargo(&root, &["test", "--workspace", "--locked"])?;
    architecture(&root)
}

fn architecture(root: &Path) -> Result<()> {
    eprintln!("==> architecture checks");
    let violations = checks::run(root)?;
    for violation in &violations {
        eprintln!("  {violation}");
    }
    if !violations.is_empty() {
        bail!("{} architecture violation(s)", violations.len());
    }
    Ok(())
}

fn cargo(root: &Path, args: &[&str]) -> Result<()> {
    eprintln!("==> cargo {}", args.join(" "));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let status = Process::new(cargo)
        .args(args)
        .current_dir(root)
        .status()
        .with_context(|| format!("running cargo {}", args[0]))?;
    if !status.success() {
        bail!("cargo {} failed", args[0]);
    }
    Ok(())
}
