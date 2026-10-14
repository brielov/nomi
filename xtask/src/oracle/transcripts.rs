//! `cargo xtask oracle transcripts`: the transcript of each command-line
//! case, stored under `expected/` for the CLI's transcript tests.
//!
//! - elm-format's cases run it in a scratch directory with its own
//!   `ELM_HOME`, on one RTS thread, so files it formats concurrently
//!   report in order.
//! - elm's cases run `elm init` and `elm install` offline, with an
//!   `ELM_HOME` whose registry and package cache hold the corpus's `elm/`
//!   packages, so `install` can build what it adds. A case runs with the
//!   elm its application names in `elm-version` if that one is pinned,
//!   else the newest, as nomi reports an outline no release reads. What elm
//!   leaves in `elm-stuff/` is its own and isn't recorded. Each package's
//!   `elm.json` is stored under `packages/`, so the CLI's tests can give
//!   nomi the same packages.
//! - elm-test's cases run elm-test 0.19.3-0, which builds with elm 0.19.3,
//!   by Node with that elm on its path and the fixture packages in an
//!   offline `ELM_HOME`.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use nomi_base::Compat;
use nomi_testkit::transcript::{Case, Transcript, cases, normalize_elm_test};

use super::modules::store_packages;
use super::sandbox::{elm_home, fetched, fixture_releases, offline};
use super::{cache_dir, fresh_dir, installed, oracles_dir, pinned_binary, pinned_elm, which};
use crate::corpus::compat;
use crate::references::References;

/// Whose transcripts to record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Tool {
    ElmFormat,
    Elm,
    ElmTest,
}

impl Tool {
    /// The directory of the tool's cases, `expected/` and the
    /// hand-written `deviations/`.
    fn cases(self) -> &'static str {
        match self {
            Tool::ElmFormat => "crates/cli/tests/elm_format",
            Tool::Elm => "crates/cli/tests/elm",
            Tool::ElmTest => "crates/cli/tests/elm_test",
        }
    }
}

/// `cargo xtask oracle transcripts`: `tool`'s, or every tool's.
pub(crate) fn run(root: &Path, refs: &References, tool: Option<Tool>) -> Result<()> {
    let tools = [Tool::ElmFormat, Tool::Elm, Tool::ElmTest];
    for tool in tools
        .into_iter()
        .filter(|t| tool.is_none_or(|only| only == *t))
    {
        let dir = root.join(tool.cases());
        match tool {
            Tool::ElmFormat => elm_format(&dir, refs)?,
            Tool::Elm => elm(root, &dir, refs)?,
            Tool::ElmTest => elm_test(&dir, refs)?,
        }
    }
    Ok(())
}

fn elm_format(dir: &Path, refs: &References) -> Result<()> {
    let binary = pinned_binary(refs, "elm-format", None)?;
    let scratch = Scratch::new("transcripts")?;
    record(dir, |_, case| {
        scratch.reset()?;
        case.create_files(&scratch.work)?;
        let mut command = Command::new(&binary);
        command.env("ELM_HOME", &scratch.home).env("GHCRTS", "-N1");
        Ok(case.run(command, &scratch.work)?)
    })
}

fn elm(root: &Path, dir: &Path, refs: &References) -> Result<()> {
    let pinned = fetched(root, |name| name.starts_with("elm/"))?;
    let compats = compat::compats(refs)?;
    let newest = *compats.last().context("no elm is pinned")?;
    store_packages(&dir.join("packages"), &pinned)?;
    let scratch = Scratch::new("elm-transcripts")?;
    record(dir, |name, case| {
        let compat = elm_version(case)
            .with_context(|| name.to_owned())?
            .filter(|c| compats.contains(c))
            .unwrap_or(newest);
        let elm = pinned_elm(refs, compat)?;
        scratch.reset()?;
        elm_home(&scratch.home, compat, &pinned)?;
        case.create_files(&scratch.work)?;
        let mut command = Command::new(&elm);
        // Relative to the case's directory, so reports that name it are
        // the same on every machine.
        command.env("ELM_HOME", Path::new("..").join("home"));
        offline(&mut command);
        let mut transcript = case.run(command, &scratch.work)?;
        transcript
            .files
            .retain(|(path, _)| !path.starts_with("elm-stuff/"));
        Ok(transcript)
    })
}

fn elm_test(dir: &Path, refs: &References) -> Result<()> {
    let scratch = Scratch::new("elm-test-transcripts")?;
    let elm_test = ElmTest::new(refs, &scratch.root.join("bin"))?;
    let fixtures = fixture_releases()?;
    record(dir, |_, case| {
        scratch.reset()?;
        elm_home(&scratch.home, Compat::V0_19_3, &fixtures)?;
        case.create_files(&scratch.work)?;
        let mut command = elm_test.command(&scratch.home);
        offline(&mut command);
        let mut transcript = case.run(command, &scratch.work)?;
        // What elm-test leaves in `elm-stuff` is its build's.
        transcript
            .files
            .retain(|(path, _)| !path.starts_with("elm-stuff/"));
        let work = scratch.work.display().to_string();
        let home = scratch.home.display().to_string();
        transcript.stdout = normalize_elm_test(&transcript.stdout, &work, &home);
        transcript.stderr = normalize_elm_test(&transcript.stderr, &work, &home);
        Ok(transcript)
    })
}

/// elm-test 0.19.3-0, run by Node with the pinned elm 0.19.3 as `elm` on
/// a `PATH` of its own.
pub(crate) struct ElmTest {
    node: PathBuf,
    runner: PathBuf,
    path: OsString,
}

impl ElmTest {
    /// elm-test, with its `elm` linked in `bin`.
    pub(crate) fn new(refs: &References, bin: &Path) -> Result<ElmTest> {
        let elm = pinned_elm(refs, Compat::V0_19_3)?;
        let runner = installed(oracles_dir()?.join("npm/node_modules/elm-test/bin/elm-test"))?;
        let node = which("node").context("node isn't on PATH")?;
        fs::create_dir_all(bin)?;
        let link = bin.join("elm");
        if !link.exists() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&elm, &link)?;
        }
        let path = std::env::join_paths([bin, node.parent().context("node's directory")?])?;
        Ok(ElmTest { node, runner, path })
    }

    /// elm-test with `home` as its `ELM_HOME`, choosing its colors itself.
    pub(crate) fn command(&self, home: &Path) -> Command {
        let mut command = Command::new(&self.node);
        command
            .arg(&self.runner)
            .env("PATH", &self.path)
            .env("ELM_HOME", home)
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR");
        command
    }
}

/// Runs each case in `dir/cases` and stores its transcript in
/// `dir/expected`.
fn record(dir: &Path, mut run: impl FnMut(&str, &Case) -> Result<Transcript>) -> Result<()> {
    let expected = dir.join("expected");
    fresh_dir(&expected)?;
    let cases = cases(&dir.join("cases"));
    eprintln!("==> {} transcripts in {}", cases.len(), dir.display());
    for (name, case) in &cases {
        let transcript = run(name, case).with_context(|| name.clone())?;
        fs::write(expected.join(format!("{name}.txt")), transcript.render())?;
    }
    Ok(())
}

/// A sandbox a tool runs each case in afresh: an `ELM_HOME` and a
/// working directory beside it.
struct Scratch {
    root: PathBuf,
    home: PathBuf,
    work: PathBuf,
}

impl Scratch {
    /// The sandbox `name` of the development cache.
    fn new(name: &str) -> Result<Scratch> {
        let root = cache_dir()?.join("sandboxes").join(name);
        Ok(Scratch {
            home: root.join("home"),
            work: root.join("work"),
            root,
        })
    }

    /// Empties the home and the working directory.
    fn reset(&self) -> Result<()> {
        fresh_dir(&self.home)?;
        fresh_dir(&self.work)
    }
}

/// The release the `elm-version` of the case's application names, if it
/// has one; none for one nomi doesn't emulate.
fn elm_version(case: &Case) -> Result<Option<Compat>> {
    let Some((_, text)) = case.files.iter().find(|(path, _)| path == "elm.json") else {
        return Ok(None);
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return Ok(None);
    };
    match (json["type"].as_str(), json["elm-version"].as_str()) {
        (Some("application"), Some(version)) => {
            let version: nomi_project::Version =
                (version.parse().ok()).with_context(|| format!("`{version}` is not a version"))?;
            Ok(version.to_string().parse().ok())
        }
        _ => Ok(None),
    }
}
