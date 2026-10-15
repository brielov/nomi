//! `cargo xtask oracle modules`: what each pinned elm reports for each
//! module-graph case, stored for the engine's report tests.
//!
//! A case is a project: `elm make`'s arguments and its files, with
//! `default.elm.json` standing in for an `elm.json` it doesn't have. Each
//! elm runs offline on a fresh copy with an `ELM_HOME` holding the
//! corpus's elm/core and elm/json, and an application's `elm-version`
//! rewritten to that elm's. The project's directory is padded to 96
//! bytes and written `$ROOT` in elm's `--report=json`, which is otherwise
//! kept as written, in `reports.tsv`; a report several elms write alike is
//! one row naming them all. What elm prints on standard error without
//! `--report`, its terminal report, goes in `terminal.tsv` the same way,
//! as a JSON string. The `docs.json` a case's `--docs` has elm write goes
//! in `docs.tsv` the same way, as written. The case's files are dated a
//! second apart in their order, since the terminal report lists modules
//! oldest first. Each package's `elm.json` is stored under `packages/`,
//! so the tests can give nomi the same packages.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, ensure};
use nomi_base::Compat;
use nomi_testkit::transcript::{Case, cases};

use super::sandbox::{
    RETRIES, Release, Retry, corpus_releases, elm_home, json_report, locked, offline, retrying,
};
use super::{
    by_output, cache_dir, fresh_dir, generated, padded_case, pinned_elm, remove_tree, utf8,
};
use crate::corpus::compat::{self, with_elm_version};
use crate::corpus::syntax::CRASH;
use crate::references::References;

const CASES: &str = "crates/engine/tests/modules";

/// The packages every case's project may depend on.
const DEPENDENCIES: [&str; 2] = ["elm/core", "elm/json"];

/// What a `docs.tsv` cell holds where elm wrote no `docs.json`.
const NO_DOCS: &str = "none";

/// How elm reports on a case.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    /// With `--report=json`.
    Json,
    /// In its terminal's words, on standard error.
    Terminal,
}

/// What one elm made of a case.
struct Outputs {
    /// Its `--report=json`, or `accepted`.
    report: String,
    /// What it printed on standard error without `--report`.
    terminal: String,
    /// The `docs.json` it wrote.
    docs: Option<String>,
}

/// `cargo xtask oracle modules`.
pub(crate) fn run(root: &Path, refs: &References) -> Result<()> {
    let dir = root.join(CASES);
    let pinned = corpus_releases(root, &DEPENDENCIES)?;
    store_packages(&dir.join("packages"), &pinned)?;
    record_projects(refs, &dir, "modules", &pinned, &cases(&dir.join("cases")))
}

/// Stores each release's `elm.json` at `dir/<author>/<project>/<version>`.
pub(super) fn store_packages(dir: &Path, pinned: &[Release]) -> Result<()> {
    remove_tree(dir)?;
    for release in pinned {
        let to = dir.join(&release.name).join(release.version.to_string());
        fs::create_dir_all(&to)?;
        fs::copy(release.dir.join("elm.json"), to.join("elm.json"))
            .with_context(|| format!("{} {}", release.name, release.version))?;
    }
    Ok(())
}

/// Records what each elm reports on each of `projects`, into `dir`'s
/// `reports.tsv` and `terminal.tsv`, with `pinned` in its `ELM_HOME` and
/// `dir`'s `default.elm.json` where a project has none. Each elm keeps one
/// `ELM_HOME`, so it builds the packages once, and the elms run at once.
pub(super) fn record_projects(
    refs: &References,
    dir: &Path,
    suite: &str,
    pinned: &[Release],
    projects: &[(String, Case)],
) -> Result<()> {
    let default = fs::read_to_string(dir.join("default.elm.json"))?;
    let compats = compat::compats(refs)?;
    let outputs: Vec<Result<Vec<Outputs>>> = std::thread::scope(|scope| {
        #[expect(
            clippy::needless_collect,
            reason = "every elm starts before the first is waited for"
        )]
        let runs: Vec<_> = (compats.iter())
            .map(|&compat| {
                let default = &default;
                scope.spawn(move || -> Result<Vec<Outputs>> {
                    let elm = Recorder::new(refs, suite, compat, pinned)?;
                    (projects.iter())
                        .map(|(id, case)| {
                            let context = || format!("elm {compat} on {id}");
                            let run = |style| elm.report(case, default, style);
                            let (report, docs) = run(Style::Json).with_context(context)?;
                            let (terminal, _) = run(Style::Terminal).with_context(context)?;
                            Ok(Outputs {
                                report,
                                terminal,
                                docs,
                            })
                        })
                        .collect()
                })
            })
            .collect();
        runs.into_iter()
            .map(|run| run.join().expect("a recorder thread"))
            .collect()
    });
    let outputs = outputs.into_iter().collect::<Result<Vec<_>>>()?;
    // Whichever suite records them, the tables name this command.
    let header = generated("modules");
    let mut out = format!("{header}\ncase\telms\treport\n");
    let mut terminal = format!("{header}\ncase\telms\tstderr\n");
    let mut docs = format!("{header}\ncase\telms\tdocs\n");
    for (i, (id, _)) in projects.iter().enumerate() {
        let made = || compats.iter().copied().zip(outputs.iter().map(|o| &o[i]));
        for (elms, report) in by_output(made().map(|(c, o)| (c, o.report.as_str()))) {
            writeln!(out, "{id}\t{}\t{report}", elms.join(","))?;
        }
        for (elms, stderr) in by_output(made().map(|(c, o)| (c, o.terminal.as_str()))) {
            let stderr = serde_json::to_string(stderr)?;
            writeln!(terminal, "{id}\t{}\t{stderr}", elms.join(","))?;
        }
        let written = made().map(|(c, o)| (c, o.docs.as_deref().unwrap_or(NO_DOCS)));
        for (elms, json) in by_output(written) {
            writeln!(docs, "{id}\t{}\t{json}", elms.join(","))?;
        }
    }
    fs::write(dir.join("reports.tsv"), out)?;
    fs::write(dir.join("terminal.tsv"), terminal)?;
    if projects
        .iter()
        .any(|(_, case)| case.args.iter().any(|a| a.starts_with("--docs=")))
    {
        fs::write(dir.join("docs.tsv"), docs)?;
    }
    println!("recorded {} cases", projects.len());
    Ok(())
}

/// Sets `elm.json` in `dir`, or `default` where the case has none, to
/// name `compat` if it's an application's; a package's stays as it is.
fn pin_manifest(dir: &Path, default: &str, compat: Compat) -> Result<()> {
    let manifest = dir.join("elm.json");
    let text = fs::read_to_string(&manifest).unwrap_or_else(|_| default.to_owned());
    let pinned = (text.contains("\"type\": \"application\""))
        .then(|| with_elm_version(&text, compat))
        .flatten();
    fs::write(&manifest, pinned.unwrap_or(text))?;
    Ok(())
}

/// One elm recording a suite's cases: the elm, a fresh scratch
/// directory for the suite's projects, and an `ELM_HOME` in it.
pub(super) struct Recorder {
    pub elm: PathBuf,
    pub scratch: PathBuf,
    pub home: PathBuf,
    pub compat: Compat,
}

impl Recorder {
    /// `compat`'s elm, recording `suite` with `pinned` in its `ELM_HOME`.
    pub(super) fn new(
        refs: &References,
        suite: &str,
        compat: Compat,
        pinned: &[Release],
    ) -> Result<Recorder> {
        let elm = pinned_elm(refs, compat)?;
        let scratch = cache_dir()?
            .join("sandboxes")
            .join(suite)
            .join(compat.as_str());
        remove_tree(&scratch)?;
        let home = scratch.join("home");
        elm_home(&home, compat, pinned)?;
        Ok(Recorder {
            elm,
            scratch,
            home,
            compat,
        })
    }

    /// elm, offline, ready to run on `case` with its arguments in a fresh
    /// project, whose `elm.json` is pinned as [`pin_manifest`] pins it;
    /// and the project.
    pub(super) fn make(&self, case: &Case, default: &str) -> Result<(Command, PathBuf)> {
        let work = self.scratch.join("work");
        fresh_dir(&work)?;
        case.create_files(&work)?;
        pin_manifest(&work, default, self.compat)?;
        Ok((self.command(&case.args, &work), work))
    }

    /// elm, offline, with `args` in `dir`.
    fn command(&self, args: &[String], dir: &Path) -> Command {
        let mut command = Command::new(&self.elm);
        command
            .args(args)
            .current_dir(dir)
            .env("ELM_HOME", &self.home);
        offline(&mut command);
        command
    }

    /// elm's report on `case` in `style`, and the `docs.json` it wrote: in
    /// JSON, its `--report=json` on one line, or `accepted`; in the
    /// terminal's, what it prints on standard error; [`CRASH`] either way
    /// where elm died by a signal. The project is made afresh, padded and
    /// dated, with `default` for an `elm.json` it lacks. A run that trips
    /// over a lock [`RETRIES`] times is recorded as it went.
    fn report(&self, case: &Case, default: &str, style: Style) -> Result<(String, Option<String>)> {
        let work = padded_case(&self.scratch.join("work"), case)?;
        pin_manifest(&work, default, self.compat)?;
        let output = retrying(RETRIES, |_, _| {
            let mut command = self.command(&case.args, &work);
            command
                .arg("--output=/dev/null")
                .args((style == Style::Json).then_some("--report=json"))
                .args(["+RTS", "-N1", "-RTS"]);
            let output = command.output().context("running elm")?;
            // elm now and then trips over its own lock on `d.dat`.
            Ok(if locked(&output.stderr) {
                Retry::Again(output)
            } else {
                Retry::Done(output)
            })
        })?;
        // 0.19.3 dies on some recursive aliases; the tests skip what crashed.
        if output.status.code().is_none() {
            return Ok((CRASH.to_owned(), None));
        }
        let docs = fs::read_to_string(work.join("docs.json")).ok();
        let root = utf8(&work)?;
        if style == Style::Terminal {
            let stderr = String::from_utf8(output.stderr)?;
            return Ok((stderr.replace(root, "$ROOT"), docs));
        }
        let Some(json) = json_report(&output) else {
            ensure!(
                output.status.success(),
                "elm exited with {} and no report:\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
            return Ok(("accepted".to_owned(), docs));
        };
        // One line: elm escapes the newlines it writes.
        let report = std::str::from_utf8(json.trim_ascii_end())?;
        Ok((report.replace(root, "$ROOT"), docs))
    }
}
