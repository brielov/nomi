//! `cargo xtask fuzz <target>`: a libFuzzer campaign over a target of
//! `fuzz/`, on stable. cargo-fuzz needs nightly for its sanitizer flags;
//! the coverage instrumentation libFuzzer steers by needs only stable
//! LLVM passes, so this builds with those and no sanitizer. Rust's own
//! checks stand in for one: debug assertions and overflow checks are on.

use std::path::{Path, PathBuf};
use std::process::Command as Process;

use anyhow::{Context, Result, bail, ensure};

/// Each target, and the inputs it starts from besides its own corpus.
const TARGETS: &[(&str, &str)] = &[
    ("syntax", "crates/syntax/tests/syntax_mutations/seeds"),
    ("format", "crates/format/tests/format_fixtures/inputs"),
    ("json", "crates/project/tests/json_seeds"),
];

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

pub fn run(root: &Path, target: &str, runs: u64, jobs: usize) -> Result<()> {
    let Some(&(_, seeds)) = TARGETS.iter().find(|(name, _)| *name == target) else {
        let names: Vec<_> = TARGETS.iter().map(|(name, _)| *name).collect();
        bail!("no fuzz target `{target}`; there are {}", names.join(", "));
    };
    ensure!(jobs > 0, "--jobs must be at least 1");
    let dir = root.join("target/fuzz");
    let host = host()?;
    eprintln!("==> building fuzz/ for {host}");
    let status = Process::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "--release", "--locked", "--manifest-path"])
        .arg(root.join("fuzz/Cargo.toml"))
        .args(["--target", &host, "--bin", target])
        .env("RUSTFLAGS", RUSTFLAGS.join(" "))
        .env("CARGO_TARGET_DIR", &dir)
        .status()
        .context("running cargo build")?;
    ensure!(status.success(), "building fuzz/ failed");

    let corpus = dir.join("corpus").join(target);
    let logs = dir.join("logs").join(target);
    let artifacts = root.join("fuzz/artifacts").join(target);
    for d in [&corpus, &artifacts] {
        std::fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
    }
    // Last campaign's logs would be summed with this one's.
    if logs.exists() {
        std::fs::remove_dir_all(&logs)?;
    }
    std::fs::create_dir_all(&logs)?;
    let before = crashes(&artifacts)?;
    let each = runs.div_ceil(jobs as u64);
    eprintln!("==> {jobs} job(s) of {each} runs each");
    let binary: PathBuf = [
        dir.as_path(),
        host.as_ref(),
        "release".as_ref(),
        target.as_ref(),
    ]
    .iter()
    .collect();
    let mut prefix = std::ffi::OsString::from("-artifact_prefix=");
    prefix.push(&artifacts);
    prefix.push("/");
    let status = Process::new(&binary)
        .arg(&corpus)
        .arg(root.join(seeds))
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
        let text = std::fs::read_to_string(entry?.path())?;
        done += text
            .lines()
            .filter_map(|l| {
                l.strip_prefix("Done ")?
                    .split(' ')
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
            .sum::<u64>();
    }
    let found: Vec<_> = crashes(&artifacts)?
        .into_iter()
        .filter(|c| !before.contains(c))
        .collect();
    if !found.is_empty() {
        bail!(
            "{done} runs found {} crash(es), in fuzz/artifacts/{target}: {}; the logs are in {}",
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
