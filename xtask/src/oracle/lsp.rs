//! `cargo xtask oracle lsp`: elm-language-server 2.10.0's transcript of
//! each language-server case (plan 9.1), stored under `expected/` for
//! `crates/lsp/tests/elm_language_server.rs` to replay against nomi.
//!
//! Each case runs in a sandbox: a copy of its project, and an `ELM_HOME`
//! whose registry and package cache hold the fixture packages
//! (`crates/compiler/tests/packages`), the same sources the replay gives
//! nomi. The server runs under Node with the pinned elm and elm-format as
//! its paths, offline. elm builds the project once before the session, so
//! the server's own `elm make` is quick and the session settles.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
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

pub fn run(root: &Path, refs: &References, only: Option<&str>) -> Result<()> {
    node_version(refs.node_minimum)?;
    let server =
        oracles_dir()?.join("npm/node_modules/@elm-tooling/elm-language-server/out/node/index.js");
    ensure!(
        server.exists(),
        "{} is missing; run `cargo xtask oracle fetch`",
        server.display()
    );
    let elm_format = binary(refs, "elm-format", None)?;
    let suite = root.join(SUITE);
    let expected = suite.join("expected");
    fs::create_dir_all(&expected)?;
    let scratch = cache_dir()?.join("sandboxes").join("lsp");
    for case in lsp::cases(&suite.join("cases")) {
        if only.is_some_and(|only| only != case.name) {
            continue;
        }
        eprintln!("  {}", case.name);
        let transcript = record(root, &server, &elm_format, refs, &scratch, &case)
            .with_context(|| format!("case {}", case.name))?;
        let text = serde_json::to_string_pretty(&transcript)? + "\n";
        fs::write(expected.join(format!("{}.json", case.name)), text)?;
    }
    Ok(())
}

fn record(
    root: &Path,
    server: &Path,
    elm_format: &Path,
    refs: &References,
    scratch: &Path,
    case: &lsp::Case,
) -> Result<serde_json::Value> {
    let dir = scratch.join(&case.name);
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    let (home, work) = (dir.join("home"), dir.join("work"));
    lsp::copy_project(case, &work)?;
    let work = fs::canonicalize(&work)?;
    let compat = Version::parse("0.19.1").context("a version")?;
    let elm = binary(refs, "elm", Some(&compat.to_string()))?;
    let packages = elm_home(root, &home, compat)?;
    let packages = fs::canonicalize(packages)?;

    // Build once, so the dependencies are built before the session.
    let modules = elm_modules(&work.join("src"))?;
    let mut make = Command::new(&elm);
    make.arg("make")
        .args(&modules)
        .arg("--output=/dev/null")
        .current_dir(&work)
        .env("ELM_HOME", &home)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    offline(&mut make).status().context("running elm make")?;

    let mut command = Command::new("node");
    command
        .arg(server)
        .arg("--stdio")
        .current_dir(&work)
        .env("ELM_HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    offline(&mut command);
    let mut child = command.spawn().context("starting elm-language-server")?;
    let connection =
        lsp::child_connection(child.stdin.take().unwrap(), child.stdout.take().unwrap());
    let mut session = case.session.clone();
    let paths = json!({
        "elmPath": elm,
        "elmFormatPath": elm_format,
        "elmTestPath": dir.join("no-elm-test"),
        "elmReviewPath": dir.join("no-elm-review"),
    });
    for (key, value) in paths.as_object().unwrap() {
        session.settings.entry(key.clone()).or_insert(value.clone());
    }
    let timing = Timing {
        quiet: Duration::from_secs(2),
        answer: Duration::from_secs(60),
    };
    let root_uri = format!("file://{}", work.display());
    let driver = Driver::start(connection, &work, &root_uri, &session, timing);
    let transcript = driver.run(&session);
    lsp::reap(child);
    let replacements = [
        (
            format!("file://{}", packages.display()),
            "<packages>".to_owned(),
        ),
        (packages.display().to_string(), "<packages>".to_owned()),
        (root_uri, "<project>".to_owned()),
        (work.display().to_string(), "<project>".to_owned()),
    ];
    Ok(lsp::normalize(transcript, &replacements))
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

/// Every `.elm` file under `dir`, sorted.
fn elm_modules(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut modules = Vec::new();
    if !dir.is_dir() {
        return Ok(modules);
    }
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            modules.extend(elm_modules(&path)?);
        } else if path.extension().is_some_and(|e| e == "elm") {
            modules.push(path);
        }
    }
    modules.sort();
    Ok(modules)
}
