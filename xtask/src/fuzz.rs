//! `cargo xtask fuzz <target>`: a libFuzzer campaign over a target of
//! `fuzz/`, on stable. cargo-fuzz needs nightly for its sanitizer flags;
//! the coverage instrumentation libFuzzer steers by needs only stable
//! LLVM passes, so this builds with those and no sanitizer. Rust's own
//! checks stand in for one: debug assertions and overflow checks are on.

use std::path::{Path, PathBuf};
use std::process::Command as Process;

use anyhow::{Context, Result, bail, ensure};

use crate::oracle::fresh_dir;

/// A target of `fuzz/`: a binary of its `Cargo.toml`.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub(crate) enum Target {
    Syntax,
    Format,
    Json,
}

impl Target {
    /// The target's binary.
    fn name(self) -> &'static str {
        match self {
            Target::Syntax => "syntax",
            Target::Format => "format",
            Target::Json => "json",
        }
    }

    /// The inputs it starts from besides its own corpus.
    fn seeds(self) -> &'static str {
        match self {
            Target::Syntax => "crates/syntax/tests/syntax_mutations/seeds",
            Target::Format => "crates/format/tests/format_fixtures/inputs",
            Target::Json => "crates/project/tests/json_seeds",
        }
    }
}

/// The flags of `cargo fuzz build --sanitizer none --debug-assertions`.
const RUSTFLAGS: &[&str] = &[
    "--cfg fuzzing",
    "-Cpasses=sancov-module",
    "-Cllvm-args=-sanitizer-coverage-level=4",
    "-Cllvm-args=-sanitizer-coverage-inline-8bit-counters",
    "-Cllvm-args=-sanitizer-coverage-pc-table",
    "-Cllvm-args=-sanitizer-coverage-trace-compares",
    "-Cdebug-assertions",
    "-Coverflow-checks",
];

/// The longest input libFuzzer makes: a screenful of Elm, past which
/// mutations mostly repeat what shorter inputs found.
const MAX_LEN: usize = 8192;

/// Runs `target` `runs` times across `jobs` fuzzer processes, failing on
/// any crash it finds.
pub(crate) fn run(root: &Path, target: Target, runs: u64, jobs: usize) -> Result<()> {
    let dir = root.join("target/fuzz");
    let binary = build(root, &dir, target)?;
    let name = target.name();
    let corpus = dir.join("corpus").join(name);
    let logs = dir.join("logs").join(name);
    let artifacts = root.join("fuzz/artifacts").join(name);
    for d in [&corpus, &artifacts] {
        std::fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
    }
    // Last campaign's logs would be summed with this one's.
    fresh_dir(&logs)?;
    let before = crashes(&artifacts)?;
    let each = runs.div_ceil(jobs as u64);
    eprintln!("==> {jobs} job(s) of {each} runs each");
    let mut prefix = std::ffi::OsString::from("-artifact_prefix=");
    prefix.push(&artifacts);
    prefix.push("/");
    let status = Process::new(&binary)
        .arg(&corpus)
        .arg(root.join(target.seeds()))
        .arg(format!("-runs={each}"))
        .arg(format!("-max_len={MAX_LEN}"))
        .arg(format!("-jobs={jobs}"))
        .arg(format!("-workers={jobs}"))
        .arg(prefix)
        // `-jobs` writes each job's `fuzz-<n>.log` to the working directory.
        .current_dir(&logs)
        .status()
        .with_context(|| format!("running {}", binary.display()))?;

    let mut done = 0;
    for entry in std::fs::read_dir(&logs)? {
        done += runs_done(&std::fs::read_to_string(entry?.path())?);
    }
    let found: Vec<_> = crashes(&artifacts)?
        .into_iter()
        .filter(|c| !before.contains(c))
        .collect();
    if !found.is_empty() {
        bail!(
            "{done} runs found {} crash(es), in fuzz/artifacts/{name}: {}; the logs are in {}",
            found.len(),
            found.join(", "),
            logs.display()
        );
    }
    ensure!(
        status.success(),
        "the fuzzer failed; the logs are in {}",
        logs.display()
    );
    ensure!(
        done >= runs,
        "only {done} of {runs} runs finished; the logs are in {}",
        logs.display()
    );
    eprintln!("{done} runs, no crashes");
    Ok(())
}

/// Builds `target` instrumented, under `dir`, and gives its binary.
fn build(root: &Path, dir: &Path, target: Target) -> Result<PathBuf> {
    let host = host()?;
    eprintln!("==> building fuzz/ for {host}");
    let status = crate::cargo_command()
        .args(["build", "--release", "--locked", "--manifest-path"])
        .arg(root.join("fuzz/Cargo.toml"))
        .args(["--target", &host, "--bin", target.name()])
        .env("RUSTFLAGS", RUSTFLAGS.join(" "))
        .env("CARGO_TARGET_DIR", dir)
        .status()
        .context("running cargo build")?;
    ensure!(status.success(), "building fuzz/ failed");
    Ok(dir.join(host).join("release").join(target.name()))
}

/// How many runs a job's log says it finished: its `Done <n> runs` lines
/// summed.
fn runs_done(log: &str) -> u64 {
    (log.lines())
        .filter_map(|l| {
            l.strip_prefix("Done ")?
                .split(' ')
                .next()?
                .parse::<u64>()
                .ok()
        })
        .sum()
}

/// The inputs libFuzzer saved as failing.
fn crashes(dir: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if ["crash-", "oom-", "timeout-", "leak-"]
            .iter()
            .any(|p| name.starts_with(p))
        {
            names.push(name);
        }
    }
    Ok(names)
}

/// The triple rustc builds for, which the instrumented build names
/// explicitly so the flags stay off build scripts.
fn host() -> Result<String> {
    let output = Process::new("rustc")
        .arg("-vV")
        .output()
        .context("running rustc -vV")?;
    String::from_utf8(output.stdout)?
        .lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(str::to_owned)
        .context("rustc -vV names no host")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_lines_add_up() {
        let log = "#1 INITED\nDone 500 runs in 2 second(s)\nnoise\nDone 25 runs in 0 second(s)\n";
        assert_eq!(runs_done(log), 525);
        assert_eq!(runs_done("Done many runs\n"), 0);
    }
}
