//! Repository automation, run as `cargo xtask <command>`.

mod checks;
mod corpus;
mod download;
mod oracle;
mod references;
mod refs;

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
    /// Sync reference/ to the source commits pinned in references.toml.
    Refs,
    /// Pinned upstream tools that parity is measured against.
    Oracle {
        #[command(subcommand)]
        command: OracleCommand,
    },
    /// Run a parity suite and record its newly passing cases in passing.txt.
    Ratchet {
        /// The suite: a test target `crates/<crate>/tests/<suite>.rs`.
        suite: String,
    },
    /// Real-world Elm code that the parity suites run over.
    Corpus {
        #[command(subcommand)]
        command: CorpusCommand,
    },
}

#[derive(Debug, Subcommand)]
enum CorpusCommand {
    /// Re-pin corpus/packages.toml from the live registry.
    Pin,
    /// Download and verify every pinned package and app.
    Fetch,
    /// Count what is pinned and fetched.
    Stats,
}

#[derive(Debug, Subcommand)]
enum OracleCommand {
    /// Download, verify and install every pinned oracle.
    Fetch,
    /// Print every pin and whether it is installed.
    Versions,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Ci => ci(),
        Command::Check => architecture(&root()),
        Command::Refs => references::load(&root()).and_then(|r| refs::sync(&root(), &r)),
        Command::Oracle { command } => references::load(&root()).and_then(|r| match command {
            OracleCommand::Fetch => oracle::fetch(&root(), &r),
            OracleCommand::Versions => oracle::versions(&r),
        }),
        Command::Ratchet { suite } => ratchet(&root(), &suite),
        Command::Corpus { command } => match command {
            CorpusCommand::Pin => corpus::packages::pin(&root()),
            CorpusCommand::Fetch => {
                corpus::packages::fetch(&root()).and_then(|()| corpus::apps::fetch(&root()))
            }
            CorpusCommand::Stats => corpus::stats(&root()),
        },
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

/// Runs the suite's test target with the ratchet allowed to record newly
/// passing cases; regressions still fail it.
fn ratchet(root: &Path, suite: &str) -> Result<()> {
    let mut owners = Vec::new();
    for entry in std::fs::read_dir(root.join("crates")).context("reading crates/")? {
        let dir = entry?.path();
        if dir.join("tests").join(format!("{suite}.rs")).is_file() {
            owners.push(dir);
        }
    }
    let [dir] = owners.as_slice() else {
        bail!(
            "expected one crates/*/tests/{suite}.rs, found {}",
            owners.len()
        );
    };
    let manifest = std::fs::read_to_string(dir.join("Cargo.toml"))?;
    let package: toml::Table = toml::from_str(&manifest)?;
    let name = package
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .context("the crate's Cargo.toml has no package name")?;
    cargo_with(
        root,
        &[
            "test",
            "--package",
            name,
            "--test",
            suite,
            "--",
            "--nocapture",
        ],
        &[("NOMI_RATCHET_UPDATE", "1")],
    )
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
    cargo_with(root, args, &[])
}

fn cargo_with(root: &Path, args: &[&str], envs: &[(&str, &str)]) -> Result<()> {
    eprintln!("==> cargo {}", args.join(" "));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let status = Process::new(cargo)
        .args(args)
        .envs(envs.iter().copied())
        .current_dir(root)
        .status()
        .with_context(|| format!("running cargo {}", args[0]))?;
    if !status.success() {
        bail!("cargo {} failed", args[0]);
    }
    Ok(())
}
