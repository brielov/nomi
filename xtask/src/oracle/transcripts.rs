//! `cargo xtask oracle transcripts`: the transcript of each command-line
//! case, stored under `expected/` for the CLI's transcript tests.
//!
//! - [`ELM_FORMAT_CASES`] run elm-format in a scratch directory with its
//!   own `ELM_HOME`, on one RTS thread, so files it formats concurrently
//!   report in order.
//! - [`ELM_CASES`] run `elm init` and `elm install` offline, with an
//!   `ELM_HOME` whose registry and package cache hold the corpus's `elm/`
//!   packages, so `install` can build what it adds. A case runs with the
//!   elm its application names in `elm-version` if that one is pinned,
//!   else the newest, as nomi reports an outline no release reads. What elm
//!   leaves in `elm-stuff/` is its own and isn't recorded. Each package's
//!   `elm.json` is stored under `packages/`, so the CLI's tests can give
//!   nomi the same packages.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, ensure};
use nomi_testkit::transcript::Case;

use super::sandbox::{elm_home, offline};
use super::{binary_path, cache_dir};
use crate::corpus::compat::{self, Version};
use crate::corpus::{corpus_dir, packages};
use crate::references::References;

/// elm-format's cases, beside `expected/` and the hand-written
/// `deviations/`.
pub const ELM_FORMAT_CASES: &str = "crates/cli/tests/elm_format";

/// elm's cases, laid out the same way.
pub const ELM_CASES: &str = "crates/cli/tests/elm";

pub fn run(root: &Path, refs: &References, tool: Option<&str>) -> Result<()> {
    if tool.is_none_or(|t| t == "elm-format") {
        elm_format(root, refs)?;
    }
    if tool.is_none_or(|t| t == "elm") {
        elm(root, refs)?;
    }
    if tool.is_none_or(|t| t == "elm-test") {
        elm_test(root, refs)?;
    }
    Ok(())
}

/// elm-test's cases, laid out the same way (plan 10.1).
pub const ELM_TEST_CASES: &str = "crates/cli/tests/elm_test";

/// elm-test 0.19.3-0, which builds with elm 0.19.3, run by Node with that
/// elm on its path and the fixture packages in an offline `ELM_HOME`.
fn elm_test(root: &Path, refs: &References) -> Result<()> {
    let version = Version::parse("0.19.3")?;
    let elm = binary(refs, "elm", Some(&version.to_string()))?;
    let runner = super::oracles_dir()?.join("npm/node_modules/elm-test/bin/elm-test");
    ensure!(
        runner.exists(),
        "{} is missing; run `cargo xtask oracle fetch`",
        runner.display()
    );
    let node = std::env::var_os("PATH")
        .and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join("node"))
                .find(|candidate| candidate.is_file())
        })
        .context("node isn't on PATH")?;
    let scratch = cache_dir()?.join("sandboxes").join("elm-test-transcripts");
    record(&root.join(ELM_TEST_CASES), |_, case| {
        let (home, work) = fresh(&scratch)?;
        super::lsp::elm_home(root, &home, version)?;
        let bin = scratch.join("bin");
        fs::create_dir_all(&bin)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&elm, bin.join("elm"))?;
        let path =
            std::env::join_paths([bin, node.parent().context("node's directory")?.to_owned()])?;
        case.create_files(&work)?;
        let mut command = Command::new(&node);
        command
            .arg(&runner)
            .env("PATH", path)
            .env("ELM_HOME", &home)
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR");
        offline(&mut command);
        let mut transcript = case.run(command, &work)?;
        // What elm-test leaves behind is its build's.
        transcript.files.clear();
        let dir = work.display().to_string();
        let home = home.display().to_string();
        transcript.stdout = normalize(&transcript.stdout, &dir, &home);
        transcript.stderr = normalize(&transcript.stderr, &dir, &home);
        Ok(transcript)
    })
}

/// elm-test's output with what changes from run to run, or machine to
/// machine, put plainly: the case's directory and `ELM_HOME` as `{dir}` and
/// `{home}`, durations as 0, the help's core count as 8, and a stack
/// trace's Node version and lines and columns in the JavaScript as 0.
pub fn normalize(output: &[u8], dir: &str, home: &str) -> Vec<u8> {
    let text = String::from_utf8_lossy(output)
        .replace(dir, "{dir}")
        .replace(home, "{home}");
    let text = digits_after(&text, "\"duration\":\"", "0");
    let text = digits_after(&text, " time=\"", "0");
    let text = digits_after(&text, "Duration: ", "0");
    // Colored, its label and number apart.
    let text = digits_after(&text, "Duration: \u{1b}[22m", "0");
    let text = digits_after(&text, "logical CPU cores, ", "8");
    // A stack trace's Node and its lines in the JavaScript.
    let text = digits_after(&text, "Node.js v", "0");
    let text = digits_after(&text, ".js:", "0");
    let text = digits_after(&text, ".js:0:", "0");
    text.into_bytes()
}

/// `text` with the number after each `marker`, digits and points, as `with`.
fn digits_after(text: &str, marker: &str, with: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(marker) {
        out.push_str(&rest[..at + marker.len()]);
        rest = &rest[at + marker.len()..];
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        if end > 0 {
            out.push_str(with);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn elm_format(root: &Path, refs: &References) -> Result<()> {
    let binary = binary(refs, "elm-format", None)?;
    let scratch = cache_dir()?.join("sandboxes").join("transcripts");
    record(&root.join(ELM_FORMAT_CASES), |_, case| {
        let (home, work) = fresh(&scratch)?;
        case.create_files(&work)?;
        let mut command = Command::new(&binary);
        command.env("ELM_HOME", &home).env("GHCRTS", "-N1");
        Ok(case.run(command, &work)?)
    })
}

fn elm(root: &Path, refs: &References) -> Result<()> {
    let pinned: Vec<packages::Package> = packages::load(root)?
        .packages
        .into_iter()
        .filter(|p| p.name.starts_with("elm/"))
        .collect();
    let compats = compat::compats(refs)?;
    let newest = *compats.last().context("no elm is pinned")?;
    fixtures(&root.join(ELM_CASES).join("packages"), &pinned)?;
    let scratch = cache_dir()?.join("sandboxes").join("elm-transcripts");
    record(&root.join(ELM_CASES), |name, case| {
        let version = elm_version(case)
            .with_context(|| name.to_owned())?
            .filter(|v| compats.contains(v))
            .unwrap_or(newest);
        let binary = binary(refs, "elm", Some(&version.to_string()))?;
        let (home, work) = fresh(&scratch)?;
        elm_home(&home, version, &pinned)?;
        case.create_files(&work)?;
        let mut command = Command::new(&binary);
        // Relative to the case's directory, so reports that name it are
        // the same on every machine.
        command.env("ELM_HOME", Path::new("..").join(home.file_name().unwrap()));
        offline(&mut command);
        let mut transcript = case.run(command, &work)?;
        transcript
            .files
            .retain(|(path, _)| !path.starts_with("elm-stuff/"));
        Ok(transcript)
    })
}

/// Stores each package's `elm.json` at `dir/<author>/<project>/<version>`.
fn fixtures(dir: &Path, pinned: &[packages::Package]) -> Result<()> {
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    let corpus = corpus_dir()?;
    for package in pinned {
        let to = dir.join(&package.name).join(&package.version);
        fs::create_dir_all(&to)?;
        fs::copy(
            packages::package_dir(&corpus, package).join("elm.json"),
            to.join("elm.json"),
        )
        .with_context(|| format!("{} {}", package.name, package.version))?;
    }
    Ok(())
}

/// Runs each case in `dir/cases` and stores its transcript in
/// `dir/expected`.
fn record(
    dir: &Path,
    mut run: impl FnMut(&str, &Case) -> Result<nomi_testkit::transcript::Transcript>,
) -> Result<()> {
    let expected = dir.join("expected");
    if expected.exists() {
        fs::remove_dir_all(&expected)?;
    }
    fs::create_dir_all(&expected)?;
    let mut names: Vec<_> = fs::read_dir(dir.join("cases"))?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_>>()?;
    names.sort();
    eprintln!("==> {} transcripts in {}", names.len(), dir.display());
    for name in &names {
        let stem = name
            .strip_suffix(".case")
            .with_context(|| format!("{name} is not a .case"))?;
        let case = Case::parse(&fs::read_to_string(dir.join("cases").join(name))?)
            .map_err(anyhow::Error::msg)
            .with_context(|| name.clone())?;
        let transcript = run(name, &case).with_context(|| name.clone())?;
        fs::write(expected.join(format!("{stem}.txt")), transcript.render())?;
    }
    Ok(())
}

/// The pinned binary named `name`, at `version` if there are several.
pub(super) fn binary(refs: &References, name: &str, version: Option<&str>) -> Result<PathBuf> {
    let pin = refs
        .binary
        .iter()
        .find(|b| b.name == name && version.is_none_or(|v| b.version == v))
        .with_context(|| format!("{name} {} is not pinned", version.unwrap_or("")))?;
    let binary = binary_path(pin)?;
    ensure!(
        binary.exists(),
        "{} is missing; run `cargo xtask oracle fetch`",
        binary.display()
    );
    Ok(binary)
}

/// An empty `ELM_HOME` and working directory under `scratch`.
fn fresh(scratch: &Path) -> Result<(PathBuf, PathBuf)> {
    let (home, work) = (scratch.join("home"), scratch.join("work"));
    if scratch.exists() {
        fs::remove_dir_all(scratch)?;
    }
    fs::create_dir_all(&home)?;
    fs::create_dir_all(&work)?;
    Ok((home, work))
}

/// The `elm-version` of the case's application, if it has one.
fn elm_version(case: &Case) -> Result<Option<Version>> {
    let Some((_, text)) = case.files.iter().find(|(path, _)| path == "elm.json") else {
        return Ok(None);
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return Ok(None);
    };
    match (json["type"].as_str(), json["elm-version"].as_str()) {
        (Some("application"), Some(version)) => Version::parse(version).map(Some),
        _ => Ok(None),
    }
}
