//! `cargo xtask oracle lsp`: elm-language-server 2.10.0's transcript of
//! each language-server case (plan 9.1), stored under `expected/` for
//! `crates/lsp/tests/elm_language_server.rs` to replay against nomi.
//!
//! Each case runs in a sandbox: a copy of its project, and an `ELM_HOME`
//! whose registry and package cache hold the fixture packages
//! (`crates/compiler/tests/packages`), the same sources the replay gives
//! nomi. The server runs under Node with the pinned elm and elm-format as
//! its paths, and as `elm` on a `PATH` without `elm-test`, offline: a
//! file no `main` reaches is compiled with `elm-test`, or `elm` from
//! `PATH` where there's none. The `ELM_HOME` is shared, and elm builds every
//! package in it once before any session, so the server's own `elm make`
//! is quick and each session settles. Sessions run side by side.

use std::fs;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use nomi_testkit::lsp::{self, Driver, Timing};
use serde_json::json;

use super::sandbox::{offline, registry_of};
use super::transcripts::binary;
use super::{cache_dir, node_version, oracles_dir};
use crate::corpus::compat::Version;
use crate::references::References;

/// The suite: `cases/`, `expected/` and `passing.txt`.
pub const SUITE: &str = "crates/lsp/tests/elm_language_server";

/// The packages every case may depend on.
const PACKAGES: &str = "crates/compiler/tests/packages";

/// What every session needs.
struct Tools {
    node: PathBuf,
    /// The server's `PATH`: the pinned elm as `elm`, as a user has it,
    /// and Node, and no `elm-test`.
    path: std::ffi::OsString,
    server: PathBuf,
    elm: PathBuf,
    elm_format: PathBuf,
    home: PathBuf,
    packages: PathBuf,
}

pub fn run(root: &Path, refs: &References, only: Option<&str>) -> Result<()> {
    node_version(refs.node_minimum)?;
    let server =
        oracles_dir()?.join("npm/node_modules/@elm-tooling/elm-language-server/out/node/index.js");
    ensure!(
        server.exists(),
        "{} is missing; run `cargo xtask oracle fetch`",
        server.display()
    );
    let compat = Version::parse("0.19.1").context("a version")?;
    let scratch = cache_dir()?.join("sandboxes").join("lsp");
    if scratch.exists() {
        fs::remove_dir_all(&scratch)?;
    }
    let home = scratch.join("home");
    let packages = fs::canonicalize(elm_home(root, &home, compat)?)?;
    let node = which("node").context("node isn't on PATH")?;
    let bin = scratch.join("bin");
    fs::create_dir_all(&bin)?;
    let elm = binary(refs, "elm", Some(&compat.to_string()))?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(&elm, bin.join("elm"))?;
    #[cfg(not(unix))]
    fs::copy(&elm, bin.join(elm.file_name().context("elm's name")?))?;
    let path = std::env::join_paths([
        bin,
        node.parent().context("node's directory")?.to_owned(),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ])?;
    let tools = Tools {
        node,
        path,
        server,
        elm: binary(refs, "elm", Some(&compat.to_string()))?,
        elm_format: binary(refs, "elm-format", None)?,
        home,
        packages,
    };
    prebuild(&tools, &scratch.join("prebuild"))?;

    let suite = root.join(SUITE);
    let expected = suite.join("expected");
    fs::create_dir_all(&expected)?;
    let mut cases = lsp::cases(&suite.join("cases"));
    cases.retain(|case| only.is_none_or(|only| case.name.starts_with(only)));
    let jobs = std::thread::available_parallelism()
        .map_or(4, |n| n.get() / 2)
        .clamp(1, 8);
    let next = AtomicUsize::new(0);
    let failures = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                while let Some(case) = cases.get(next.fetch_add(1, Ordering::Relaxed)) {
                    // The driver panics on a broken session; that fails
                    // the case, not the run.
                    let recorded = std::panic::catch_unwind(AssertUnwindSafe(|| {
                        record(&tools, &scratch.join("cases"), case)
                    }))
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("the session broke")));
                    let written = recorded.and_then(|t| {
                        let text = lsp::to_text(&t);
                        Ok(fs::write(
                            expected.join(format!("{}.json", case.name)),
                            text,
                        )?)
                    });
                    if let Err(e) = written {
                        failures
                            .lock()
                            .unwrap()
                            .push(format!("{}: {e:#}", case.name));
                    }
                }
            });
        }
    });
    eprintln!("  {} cases recorded", cases.len());
    let failures = failures.into_inner().unwrap();
    if !failures.is_empty() {
        bail!("{} cases failed:\n{}", failures.len(), failures.join("\n"));
    }
    Ok(())
}

/// The first `name` on `PATH`.
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Builds every fixture package into the shared `ELM_HOME`, through an
/// application that depends on them all.
fn prebuild(tools: &Tools, dir: &Path) -> Result<()> {
    fs::create_dir_all(dir.join("src"))?;
    fs::write(dir.join("elm.json"), lsp::DEFAULT_ELM_JSON)?;
    fs::write(
        dir.join("src/Main.elm"),
        "module Main exposing (zero)\n\n\nzero =\n    0\n",
    )?;
    let mut make = Command::new(&tools.elm);
    make.args(["make", "src/Main.elm", "--output=/dev/null"])
        .current_dir(dir)
        .env("ELM_HOME", &tools.home)
        .stdout(Stdio::null());
    let status = offline(&mut make).status().context("running elm make")?;
    ensure!(status.success(), "elm couldn't build the fixture packages");
    Ok(())
}

fn record(tools: &Tools, scratch: &Path, case: &lsp::Case) -> Result<serde_json::Value> {
    fs::create_dir_all(scratch)?;
    let work = lsp::project_dir(&fs::canonicalize(scratch)?, &case.name);
    lsp::copy_project(case, &work)?;

    let mut command = Command::new(&tools.node);
    command
        .arg(&tools.server)
        .env("PATH", &tools.path)
        .arg("--stdio")
        .current_dir(&work)
        .env("ELM_HOME", &tools.home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    offline(&mut command);
    let mut child = command.spawn().context("starting elm-language-server")?;
    let connection =
        lsp::child_connection(child.stdin.take().unwrap(), child.stdout.take().unwrap());
    let mut session = case.session.clone();
    let paths = json!({
        "elmPath": tools.elm,
        "elmFormatPath": tools.elm_format,
        // Unset, so it looks for `elm-test`, finds none, and falls back
        // to `elm`, as it does for most users.
        "elmTestPath": "",
        "elmReviewPath": scratch.join("no-elm-review"),
    });
    for (key, value) in paths.as_object().unwrap() {
        session.settings.entry(key.clone()).or_insert(value.clone());
    }
    let timing = Timing {
        quiet: Duration::from_secs(2),
        answer: Duration::from_secs(60),
        // It publishes only what changed, so silence alone tells.
        publishes: false,
    };
    let root_uri = format!("file://{}", work.display());
    let driver = Driver::start(connection, &work, &root_uri, &session, timing);
    let transcript = driver.run(&session);
    lsp::reap(child);
    let packages = &tools.packages;
    let replacements = [
        (
            format!("file://{}", packages.display()),
            "<packages>".to_owned(),
        ),
        (packages.display().to_string(), "<packages>".to_owned()),
        (root_uri, "<project>".to_owned()),
        (work.display().to_string(), "<project>".to_owned()),
    ];
    Ok(lsp::tidy(transcript, &replacements))
}

/// Fills `home` as an offline `ELM_HOME` holding the fixture packages,
/// and gives back their directory.
fn elm_home(root: &Path, home: &Path, compat: Version) -> Result<PathBuf> {
    let packages = home.join(compat.to_string()).join("packages");
    let fixtures = root.join(PACKAGES);
    let mut releases = Vec::new();
    for author in fs::read_dir(&fixtures)? {
        let author = author?;
        if !author.file_type()?.is_dir() {
            continue;
        }
        for project in fs::read_dir(author.path())? {
            let project = project?;
            for version in fs::read_dir(project.path())? {
                let version = version?;
                let name = format!(
                    "{}/{}",
                    author.file_name().to_string_lossy(),
                    project.file_name().to_string_lossy()
                );
                let number = version.file_name().to_string_lossy().into_owned();
                copy_tree(&version.path(), &packages.join(&name).join(&number))?;
                releases.push((name, Version::parse(&number).context("a version")?));
            }
        }
    }
    let releases: Vec<(&str, Version)> = releases.iter().map(|(n, v)| (n.as_str(), *v)).collect();
    fs::write(
        packages.join("registry.dat"),
        registry_of(compat, &releases),
    )?;
    Ok(packages)
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), to.join(entry.file_name()))?;
        }
    }
    Ok(())
}
