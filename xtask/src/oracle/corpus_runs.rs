//! What each pinned elm makes of every corpus project, offline, for the
//! tier-2 suites to compare nomi with: `cargo xtask oracle corpus-docs`
//! runs `elm make --docs` on every package, and `cargo xtask
//! oracle corpus-check` runs `elm make --output=/dev/null` on every app
//! and package; `corpus-graphs` and `corpus-objects` keep
//! the optimized graphs that check makes; and
//! `corpus-javascript` builds every app in dev, `--debug` and
//! `--optimize`, a cell holding each output's hash, or `-` for a mode elm
//! refuses.
//!
//! Each elm runs offline with its own `ELM_HOME`, which holds every
//! release the development cache has downloaded, as `cargo xtask corpus
//! names` downloads them, and a registry listing exactly those; so run
//! that suite first. The home is kept between runs, with the
//! dependencies elm built in it, until the cache's releases change. A
//! project gets a fresh copy of its `elm.json` and modules; an app's
//! `elm.json` names the Compat, as the corpus pins it. A cell holds
//! `ok` for silence, the hash of the `docs.json` elm wrote, or of its
//! `--report=json` with the project's directory written `$ROOT`, and the
//! first title; a report about the project rather than its modules, such
//! as a dependency the cache lacks, is kept by title, for the suite to
//! skip. The outputs themselves stay in the sandbox, for looking into a
//! mismatch.

use std::fmt::{self, Write as _};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use nomi_base::Compat;
use nomi_testkit::files_below;
use nomi_testkit::sha256_hex as sha256;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use super::javascript::Mode;
use super::process::run_to_files;
use super::sandbox::{Release, elm_home, elmo_files, first_title, json_report, locked, offline};
use super::{cache_dir, cores, elm_header, generated, pinned_elm, remove_tree, utf8};
use crate::corpus::compat;
use crate::corpus::syntax::CRASH;
use crate::corpus::{apps, corpus_dir, packages, parallel_on};
use crate::references::References;

/// Runs of one project before a lock failure is an error.
const RETRIES: u32 = 5;

/// How long one project may take before elm counts as hung.
const TIMEOUT: Duration = Duration::from_secs(600);

/// Which `elm make` a suite records.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Suite {
    /// `elm make --docs` on every package.
    Docs,
    /// `elm make --output=/dev/null` on every app and package.
    Check,
    /// The same, keeping the `.elmo` of every module elm built.
    Graphs,
    /// The same, keeping the dependencies' `o.dat`.
    Objects,
    /// Every app's JavaScript in each mode.
    JavaScript,
}

impl Suite {
    /// The suite's name: `cargo xtask corpus <name>` checks what `cargo
    /// xtask oracle corpus-<name>` records in `corpus/<name>/`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Suite::Docs => "docs",
            Suite::Check => "check",
            Suite::Graphs => "graphs",
            Suite::Objects => "objects",
            Suite::JavaScript => "javascript",
        }
    }

    /// The oracle's file, from the repository's root.
    pub(crate) fn file(self) -> String {
        format!("corpus/{}/oracle.tsv", self.name())
    }

    /// The projects the suite runs elm on.
    pub(crate) fn projects(self) -> Projects {
        match self {
            Suite::Docs => Projects::Packages,
            Suite::JavaScript => Projects::Apps,
            Suite::Check | Suite::Graphs | Suite::Objects => Projects::All,
        }
    }

    /// Where elm runs and keeps what it made, for `compat`.
    pub(crate) fn sandbox(self, compat: Compat) -> Result<PathBuf> {
        Ok((cache_dir()?.join("sandboxes/corpus"))
            .join(format!("corpus-{}", self.name()))
            .join(compat.as_str()))
    }
}

/// Which corpus projects a suite takes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Projects {
    All,
    Apps,
    Packages,
}

impl Projects {
    pub(crate) fn apps(self) -> bool {
        self != Projects::Packages
    }

    pub(crate) fn packages(self) -> bool {
        self != Projects::Apps
    }
}

/// What elm made of a project, as an oracle's cell holds it, or what nomi
/// made of it as elm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Cell {
    /// `-`: the Compat doesn't accept the package.
    Excluded,
    /// `ok`: made without a word.
    Ok,
    /// `ok-noisy <hash>`: made, saying something; the hash of its standard
    /// output.
    Noisy(String),
    /// `crash`: elm died without a report, which gives no verdict.
    Crash,
    /// `docs <hash>`: the hash of its `docs.json`.
    Docs(String),
    /// `graphs <digest> <count>`: its modules' graphs, by [`graphs_cell`].
    Graphs { digest: String, count: usize },
    /// `objects <hash>`: the hash of its dependencies' `o.dat`.
    Objects(String),
    /// `javascript <dev> <debug> <optimize>`: the hash of the JavaScript
    /// of each [`Mode`], or `-` where elm refuses the mode.
    JavaScript([Option<String>; 3]),
    /// `errors <hash> <title>`: a report about modules, by the hash of its
    /// `--report=json` with the project's directory written `$ROOT`, and
    /// its first title.
    Errors { hash: String, title: String },
    /// `error <title>`: a report about the project rather than its
    /// modules, such as a dependency the cache lacks.
    Error(String),
}

impl Cell {
    /// The cell of elm's `--report=json`.
    pub(crate) fn of_report(report: &str) -> Cell {
        let value: Value = serde_json::from_str(report).unwrap_or(Value::Null);
        let title = first_title(&value).to_owned();
        if value["type"] == "compile-errors" {
            let hash = sha256(report.as_bytes());
            Cell::Errors { hash, title }
        } else {
            Cell::Error(title)
        }
    }
}

impl fmt::Display for Cell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Cell::Excluded => f.write_str("-"),
            Cell::Ok => f.write_str("ok"),
            Cell::Noisy(hash) => write!(f, "ok-noisy {hash}"),
            Cell::Crash => f.write_str(CRASH),
            Cell::Docs(hash) => write!(f, "docs {hash}"),
            Cell::Graphs { digest, count } => write!(f, "graphs {digest} {count}"),
            Cell::Objects(hash) => write!(f, "objects {hash}"),
            Cell::JavaScript(modes) => {
                f.write_str("javascript")?;
                for hash in modes {
                    write!(f, " {}", hash.as_deref().unwrap_or("-"))?;
                }
                Ok(())
            }
            Cell::Errors { hash, title } => write!(f, "errors {hash} {title}"),
            Cell::Error(title) => write!(f, "error {title}"),
        }
    }
}

impl FromStr for Cell {
    type Err = anyhow::Error;

    fn from_str(cell: &str) -> Result<Cell> {
        let malformed = || anyhow::anyhow!("malformed cell `{cell}`");
        let (word, rest) = cell.split_once(' ').unwrap_or((cell, ""));
        Ok(match word {
            "-" if rest.is_empty() => Cell::Excluded,
            "ok" if rest.is_empty() => Cell::Ok,
            CRASH if rest.is_empty() => Cell::Crash,
            "ok-noisy" => Cell::Noisy(rest.to_owned()),
            "docs" => Cell::Docs(rest.to_owned()),
            "objects" => Cell::Objects(rest.to_owned()),
            "graphs" => {
                let (digest, count) = rest.split_once(' ').ok_or_else(malformed)?;
                let count = count.parse().map_err(|_| malformed())?;
                Cell::Graphs {
                    digest: digest.to_owned(),
                    count,
                }
            }
            "javascript" => {
                let modes: Vec<Option<String>> = (rest.split(' '))
                    .map(|hash| (hash != "-").then(|| hash.to_owned()))
                    .collect();
                Cell::JavaScript(modes.try_into().map_err(|_| malformed())?)
            }
            "errors" => {
                let (hash, title) = rest.split_once(' ').ok_or_else(malformed)?;
                Cell::Errors {
                    hash: hash.to_owned(),
                    title: title.to_owned(),
                }
            }
            "error" => Cell::Error(rest.to_owned()),
            _ => bail!(malformed()),
        })
    }
}

/// A project to run elm on: its case, the directory to copy it from, and
/// what it is.
struct Project {
    case: String,
    top: PathBuf,
    kind: Kind,
}

enum Kind {
    App(Box<apps::App>),
    Package(packages::Package),
}

impl Project {
    /// Its root, within the copy.
    fn root(&self) -> &str {
        match &self.kind {
            Kind::App(app) => &app.root,
            Kind::Package(_) => ".",
        }
    }
}

/// A project's name in the sandbox, which it names its directories and
/// files by.
pub(crate) fn sandbox_id(case: &str) -> String {
    case.replace('/', "__")
}

/// Records every corpus project of `suite`, or with `only`, prints the
/// cells of the projects whose case contains it.
pub(crate) fn run(
    root: &Path,
    refs: &References,
    suite: Suite,
    jobs: Option<usize>,
    only: Option<&str>,
) -> Result<()> {
    let classification = compat::load(root)?;
    let releases = releases(&cache_dir()?.join("nomi").join("packages"))?;
    ensure!(
        !releases.is_empty(),
        "the development cache has no releases; run `cargo xtask corpus names`"
    );
    let mut projects = projects(root, suite.projects())?;
    projects.retain(|p| only.is_none_or(|o| p.case.contains(o)));
    let compats = compat::compats(refs)?;
    let mut columns = Vec::new();
    for &compat in &compats {
        let home = cache_dir()?
            .join("sandboxes/corpus/home")
            .join(compat.as_str());
        elm_home(&home, compat, &releases)?;
        let scratch = suite.sandbox(compat)?;
        remove_tree(&scratch)?;
        let elm = pinned_elm(refs, compat)?;
        let cells = parallel_on(jobs.unwrap_or_else(cores), &projects, |project| {
            let manifest = match &project.kind {
                Kind::App(app) => compat::sandbox_manifest(&classification, app, compat)?,
                Kind::Package(p) if classification.admits(&p.name, &p.version, compat) => {
                    fs::read_to_string(project.top.join("elm.json"))?
                }
                Kind::Package(_) => return Ok(Cell::Excluded),
            };
            record(suite, &elm, &home, &scratch, project, &manifest)
                .with_context(|| format!("elm {compat} on {}", project.case))
        });
        let cells = cells.into_iter().collect::<Result<Vec<_>>>()?;
        eprintln!("  elm {compat}: {} projects", cells.len());
        columns.push(cells);
    }
    let header = elm_header("case", compats.iter().copied());
    let mut out = format!(
        "{}\n{header}\n",
        generated(&format!("corpus-{}", suite.name()))
    );
    for (i, project) in projects.iter().enumerate() {
        write!(out, "{}", project.case)?;
        for column in &columns {
            write!(out, "\t{}", column[i])?;
        }
        out.push('\n');
    }
    if only.is_some() {
        print!("{out}");
        return Ok(());
    }
    let path = root.join(suite.file());
    fs::create_dir_all(path.parent().expect("a file in a directory"))?;
    fs::write(path, out)?;
    println!("recorded {} projects", projects.len());
    Ok(())
}

/// The corpus's apps, then its packages that have an `elm.json`, as
/// `projects` takes them.
fn projects(root: &Path, projects: Projects) -> Result<Vec<Project>> {
    let corpus = corpus_dir()?;
    let mut found = Vec::new();
    if projects.apps() {
        for app in apps::load(root)?.app {
            found.push(Project {
                case: format!("app/{}", app.name),
                top: apps::app_dir(&corpus, &app),
                kind: Kind::App(Box::new(app)),
            });
        }
    }
    if projects.packages() {
        for package in packages::load(root)?.packages {
            let top = packages::package_dir(&corpus, &package);
            if top.join("elm.json").exists() {
                found.push(Project {
                    case: format!("{}@{}", package.name, package.version),
                    top,
                    kind: Kind::Package(package),
                });
            }
        }
    }
    Ok(found)
}

/// Every release in nomi's store under `store`, with its sources.
pub(crate) fn releases(store: &Path) -> Result<Vec<Release>> {
    let mut releases = Vec::new();
    let dir = store.join("releases");
    if !dir.is_dir() {
        return Ok(releases);
    }
    for file in files_below(&dir)? {
        let parts: Vec<&str> = file.split('/').collect();
        let [author, project, version] = parts[..] else {
            continue;
        };
        let hash = fs::read_to_string(dir.join(&file))?;
        let sources = store.join("sources").join(hash.trim());
        if !sources.join("elm.json").exists() {
            continue;
        }
        let Ok(version) = version.parse() else {
            continue;
        };
        releases.push(Release {
            name: format!("{author}/{project}"),
            version,
            dir: sources,
        });
    }
    releases.sort_by(|a, b| (&a.name, a.version).cmp(&(&b.name, b.version)));
    Ok(releases)
}

/// The cell for `suite`'s `elm make` on `project`, with `manifest` for
/// its `elm.json`.
fn record(
    suite: Suite,
    elm: &Path,
    home: &Path,
    scratch: &Path,
    project: &Project,
    manifest: &str,
) -> Result<Cell> {
    let id = sandbox_id(&project.case);
    let work = scratch.join("work").join(&id);
    remove_tree(&work)?;
    copy_modules(&project.top, &work)?;
    let dir = work.join(project.root());
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("elm.json"), manifest)?;
    let mut args: Vec<&str> = match (suite, &project.kind) {
        (Suite::Docs, _) => vec!["--docs=docs.json"],
        (_, Kind::App(app)) => app.entries.iter().map(String::as_str).collect(),
        (_, Kind::Package(_)) => Vec::new(),
    };
    args.push("--report=json");
    args.push(match suite {
        Suite::JavaScript => "--output=out.js",
        Suite::Docs | Suite::Check | Suite::Graphs | Suite::Objects => "--output=/dev/null",
    });
    let output = make(elm, home, &work, &dir, &args)?;
    if output.status.code().is_none() {
        return Ok(Cell::Crash);
    }
    let out = scratch.join("out");
    fs::create_dir_all(&out)?;
    let keep = |file: &str, bytes: &[u8]| fs::write(out.join(format!("{id}.{file}")), bytes);
    let stuff = || -> Result<PathBuf> { Ok(dir.join("elm-stuff").join(version_dir(&dir)?)) };
    let made = match suite {
        Suite::Docs => match fs::read(dir.join("docs.json")) {
            Ok(docs) => {
                keep("docs.json", &docs)?;
                Some(Cell::Docs(sha256(&docs)))
            }
            Err(_) => None,
        },
        _ if !output.status.success() => None,
        Suite::Check => {
            keep("stdout", &output.stdout)?;
            keep("stderr", &output.stderr)?;
            Some(if output.stdout.is_empty() && output.stderr.is_empty() {
                Cell::Ok
            } else {
                Cell::Noisy(sha256(&output.stdout))
            })
        }
        Suite::Graphs => {
            let stuff = stuff()?;
            let kept = out.join(&id);
            fs::create_dir_all(&kept)?;
            let mut modules = Vec::new();
            for (module, file) in elmo_files(&stuff)? {
                let bytes = fs::read(stuff.join(&file))?;
                fs::write(kept.join(&file), &bytes)?;
                modules.push((module, bytes));
            }
            Some(graphs_cell(&modules))
        }
        Suite::Objects => {
            let objects = fs::read(stuff()?.join("o.dat"))?;
            keep("o.dat", &objects)?;
            Some(Cell::Objects(sha256(&objects)))
        }
        Suite::JavaScript => {
            let mut hashes = Vec::new();
            for mode in Mode::ALL {
                let output = match mode.flag() {
                    None => output.clone(),
                    Some(flag) => make(elm, home, &work, &dir, &[&args[..], &[flag]].concat())?,
                };
                let mode = mode.name();
                if output.status.success() {
                    let js = fs::read(dir.join("out.js"))?;
                    keep(&format!("{mode}.js"), &js)?;
                    hashes.push(Some(sha256(&js)));
                } else {
                    keep(&format!("{mode}.report.json"), &output.stderr)?;
                    hashes.push(None);
                }
            }
            Some(Cell::JavaScript(hashes.try_into().expect("a hash a mode")))
        }
    };
    let cell = match (made, json_report(&output)) {
        (Some(cell), _) => cell,
        (None, Some(json)) => {
            let root = fs::canonicalize(&dir)?;
            let report = std::str::from_utf8(json.trim_ascii_end())?.replace(utf8(&root)?, "$ROOT");
            keep("report.json", report.as_bytes())?;
            Cell::of_report(&report)
        }
        // Still dying without a report: kept for a look, and skipped.
        (None, None) => {
            keep("stderr", &output.stderr)?;
            Cell::Crash
        }
    };
    fs::remove_dir_all(&work)?;
    Ok(cell)
}

/// `elm make` with `args` in `dir`, again while it fails as runs that
/// share an `ELM_HOME` sometimes make it fail.
fn make(elm: &Path, home: &Path, work: &Path, dir: &Path, args: &[&str]) -> Result<Output> {
    // Runs sharing an `ELM_HOME` may build the same dependency at once,
    // and one may read the other's `artifacts.dat` half written; elm then
    // dies without a report, and a later run finds the file whole.
    let mut attempt = 1;
    loop {
        let mut command = Command::new(elm);
        command
            .arg("make")
            .args(args)
            .args(["+RTS", "-N1", "-RTS"])
            .current_dir(dir)
            .env("ELM_HOME", home);
        let Some(output) = run_to_files(offline(&mut command), work, TIMEOUT)? else {
            // Hung, as concurrent runs very rarely leave elm; again.
            attempt += 1;
            ensure!(attempt <= RETRIES, "elm hung {RETRIES} times");
            continue;
        };
        let unreported = !output.status.success() && json_report(&output).is_none();
        // A success that says something has, so far, been one of those
        // too: alone, it is silent.
        let noisy = output.status.success() && !output.stderr.is_empty();
        let retry = locked(&output.stderr) || unreported || noisy;
        if !retry || attempt == RETRIES {
            return Ok(output);
        }
        attempt += 1;
        let _ = fs::remove_dir_all(dir.join("elm-stuff"));
        std::thread::sleep(Duration::from_secs(u64::from(attempt)));
    }
}

/// Copies `top`'s `elm.json` files and modules, which are all elm reads,
/// to `work`.
pub(crate) fn copy_modules(top: &Path, work: &Path) -> Result<()> {
    for file in files_below(top)? {
        let wanted = file.ends_with(".elm") || file.ends_with(".js") && file.contains("Kernel");
        if !wanted || file.starts_with(".git/") || file.contains("node_modules/") {
            continue;
        }
        let target = work.join(&file);
        fs::create_dir_all(target.parent().expect("a file in a directory"))?;
        fs::copy(top.join(&file), &target)?;
    }
    Ok(())
}

/// The `elm-stuff` directory elm made in `dir`, named after its release.
fn version_dir(dir: &Path) -> Result<String> {
    let entry = fs::read_dir(dir.join("elm-stuff"))?
        .next()
        .context("elm left no elm-stuff")??;
    Ok(entry.file_name().to_string_lossy().into_owned())
}

/// The cell for a project's module graphs, each a module's name and the
/// bytes of its `.elmo`: the digest over each name and the SHA-256 of its
/// bytes, in the order of names, and their count.
pub(crate) fn graphs_cell(modules: &[(String, Vec<u8>)]) -> Cell {
    let mut sorted: Vec<&(String, Vec<u8>)> = modules.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut listing = Sha256::new();
    for (name, bytes) in sorted {
        listing.update(format!("{name} {}\n", sha256(bytes)));
    }
    Cell::Graphs {
        digest: nomi_base::hex(&listing.finalize()),
        count: modules.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_read_back_as_written() {
        let hash = || "ab".repeat(32);
        for cell in [
            Cell::Excluded,
            Cell::Ok,
            Cell::Noisy(hash()),
            Cell::Crash,
            Cell::Docs(hash()),
            Cell::Graphs {
                digest: hash(),
                count: 3,
            },
            Cell::Objects(hash()),
            Cell::JavaScript([Some(hash()), None, Some(hash())]),
            Cell::Errors {
                hash: hash(),
                title: "MODULE NOT FOUND".to_owned(),
            },
            Cell::Error("MISSING SOURCE DIRECTORY".to_owned()),
        ] {
            assert_eq!(cell.to_string().parse::<Cell>().unwrap(), cell);
        }
        for malformed in ["", "ok ok", "graphs ab", "javascript ab -", "nonsense"] {
            assert!(malformed.parse::<Cell>().is_err(), "{malformed}");
        }
    }

    #[test]
    fn graphs_are_digested_in_the_order_of_names() {
        let (a, b) = (
            ("A".to_owned(), b"a".to_vec()),
            ("B".to_owned(), b"b".to_vec()),
        );
        let listing = format!("A {}\nB {}\n", sha256(b"a"), sha256(b"b"));
        let expected = Cell::Graphs {
            digest: sha256(listing.as_bytes()),
            count: 2,
        };
        assert_eq!(graphs_cell(&[b, a]), expected);
    }
}
