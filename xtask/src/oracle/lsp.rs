//! `cargo xtask oracle lsp`: elm-language-server 2.10.0's transcript of
//! each language-server case, stored under `expected/` for
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

use std::ffi::OsString;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use nomi_base::Compat;
use nomi_testkit::lsp::{self, Driver, Timing};
use serde_json::{Map, Value, json};

use super::sandbox::{elm_home, fixture_releases, offline};
use super::{ElmOnPath, elm_on_path, remove_tree};
use super::{cache_dir, cores, installed, node_version, oracles_dir, pinned_binary};
use crate::references::References;
use nomi_testkit::parallel_on;

/// The suite: `cases/`, `expected/` and `passing.txt`.
pub(crate) const SUITE: &str = "crates/lsp/tests/elm_language_server";

/// The elm the server runs with, as most of its users have it.
const COMPAT: Compat = Compat::V0_19_1;

/// What every session needs.
pub(crate) struct Tools {
    pub node: PathBuf,
    /// The server's `PATH`: the pinned elm as `elm`, as a user has it,
    /// and Node, and no `elm-test`.
    pub path: OsString,
    /// The server's entry point.
    pub server: PathBuf,
    elm: PathBuf,
    elm_format: PathBuf,
    pub home: PathBuf,
    /// The package cache in `home`.
    packages: PathBuf,
}

/// `cargo xtask oracle lsp`: the cases whose names start with `only`, or
/// every case.
pub(crate) fn run(root: &Path, refs: &References, only: Option<&str>) -> Result<()> {
    node_version(refs.node_minimum)?;
    let server = server_path()?;
    let scratch = cache_dir()?.join("sandboxes").join("lsp");
    remove_tree(&scratch)?;
    let home = scratch.join("home");
    let packages = fs::canonicalize(elm_home(&home, COMPAT, &fixture_releases()?)?)?;
    let tools = tools(refs, server, &scratch, home, packages)?;
    prebuild(&tools, &scratch.join("prebuild"))?;

    let suite = root.join(SUITE);
    let expected = suite.join("expected");
    fs::create_dir_all(&expected)?;
    let mut cases = lsp::cases(&suite.join("cases"));
    cases.retain(|case| only.is_none_or(|only| case.name.starts_with(only)));
    let written = parallel_on((cores() / 2).clamp(1, 8), &cases, |case| -> Result<()> {
        let transcript = record(&tools, &scratch.join("cases"), case)?;
        let path = expected.join(format!("{}.json", case.name));
        Ok(fs::write(path, lsp::to_text(&transcript))?)
    });
    eprintln!("  {} cases recorded", cases.len());
    let failures: Vec<String> = (cases.iter().zip(written))
        .filter_map(|(case, written)| Some(format!("{}: {:#}", case.name, written.err()?)))
        .collect();
    if !failures.is_empty() {
        bail!("{} cases failed:\n{}", failures.len(), failures.join("\n"));
    }
    Ok(())
}

/// What every session needs, with `home` as the `ELM_HOME` whose package
/// cache is `packages`, and a `PATH` of its own under `scratch`.
pub(crate) fn tools(
    refs: &References,
    server: PathBuf,
    scratch: &Path,
    home: PathBuf,
    packages: PathBuf,
) -> Result<Tools> {
    let system = [Path::new("/usr/bin"), Path::new("/bin")];
    let ElmOnPath { node, elm, path } = elm_on_path(refs, COMPAT, &scratch.join("bin"), &system)?;
    Ok(Tools {
        node,
        path,
        server,
        elm,
        elm_format: pinned_binary(refs, "elm-format", None)?,
        home,
        packages,
    })
}

impl Tools {
    /// The server, started as an editor starts it: by Node, with its
    /// `PATH` and `ELM_HOME`, on standard streams.
    pub(crate) fn server_command(&self) -> Command {
        let mut command = Command::new(&self.node);
        command
            .arg(&self.server)
            .arg("--stdio")
            .env("PATH", &self.path)
            .env("ELM_HOME", &self.home);
        command
    }

    /// The settings that point the server at the pinned tools, as the
    /// recordings give them, with `elm-review` at `no_review`.
    pub(crate) fn settings(&self, no_review: &Path) -> Map<String, Value> {
        let settings = [
            ("elmPath", json!(self.elm)),
            ("elmFormatPath", json!(self.elm_format)),
            // Unset, so it looks for `elm-test`, finds none, and falls
            // back to `elm`, as it does for most users.
            ("elmTestPath", json!("")),
            ("elmReviewPath", json!(no_review)),
        ];
        Map::from_iter(settings.map(|(key, value)| (key.to_owned(), value)))
    }
}

/// The pinned elm-language-server's entry point.
pub(crate) fn server_path() -> Result<PathBuf> {
    installed(
        oracles_dir()?.join("npm/node_modules/@elm-tooling/elm-language-server/out/node/index.js"),
    )
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

/// The server's transcript of `case`, run in a copy of its project under
/// `scratch`.
pub(crate) fn record(tools: &Tools, scratch: &Path, case: &lsp::Case) -> Result<Value> {
    fs::create_dir_all(scratch)?;
    let work = lsp::project_dir(&fs::canonicalize(scratch)?, &case.name)?;
    // Nothing of an earlier run's copy stays.
    remove_tree(&work)?;
    lsp::copy_project(case, &work)?;

    let mut command = tools.server_command();
    command
        .current_dir(&work)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    offline(&mut command);
    let mut child = command.spawn().context("starting elm-language-server")?;
    let connection = lsp::child_connection(
        child.stdin.take().expect("piped"),
        child.stdout.take().expect("piped"),
    );
    let mut session = case.session.clone();
    for (key, value) in tools.settings(&scratch.join("no-elm-review")) {
        session.settings.entry(key).or_insert(value);
    }
    let timing = Timing {
        quiet: Duration::from_secs(2),
        answer: Duration::from_secs(60),
        // It publishes only what changed, so silence alone tells.
        publishes: false,
    };
    // The driver panics on a broken session; the server is reaped either
    // way.
    let transcript = panic::catch_unwind(AssertUnwindSafe(|| {
        Driver::start(connection, &work, &session, timing).run(&session)
    }));
    lsp::reap(child);
    let transcript = transcript.map_err(|_| anyhow!("the session broke"))?;
    let mut replacements = lsp::dir_replacements(&[&tools.packages], "<packages>");
    // It writes URIs percent-encoded, so `@` as `%40` in a package's copy.
    let encoded = lsp::dir_uri(&work).replace('@', "%40");
    replacements.push((encoded, "<project>".to_owned()));
    replacements.extend(lsp::dir_replacements(&[&work], "<project>"));
    Ok(lsp::tidy(transcript, &replacements))
}
