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
//! `elm.json` names the Compat, as in the 1.15a sandboxes. A cell holds
//! `ok` for silence, the hash of the `docs.json` elm wrote, or of its
//! `--report=json` with the project's directory written `$ROOT`, and the
//! first title; a report about the project rather than its modules, such
//! as a dependency the cache lacks, is kept by title, for the suite to
//! skip. The outputs themselves stay in the sandbox, for looking into a
//! mismatch.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use super::sandbox::{offline, registry_of};
use super::{binary_path, cache_dir};
use crate::corpus::compat::{self, Version};
use crate::corpus::{apps, corpus_dir, files_below, packages, parallel_on};
use crate::download::sha256;
use crate::references::References;

/// What a cell holds for a Compat that doesn't accept the package.
pub(crate) const EXCLUDED: &str = "-";

/// What a cell holds when elm made the project without a word.
pub(crate) const OK: &str = "ok";

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
    /// The oracle's file, from the repository's root.
    pub(crate) fn file(self) -> &'static str {
        match self {
            Suite::Docs => "corpus/docs/oracle.tsv",
            Suite::Check => "corpus/check/oracle.tsv",
            Suite::Graphs => "corpus/graphs/oracle.tsv",
            Suite::Objects => "corpus/objects/oracle.tsv",
            Suite::JavaScript => "corpus/javascript/oracle.tsv",
        }
    }

    fn command(self) -> &'static str {
        match self {
            Suite::Docs => "corpus-docs",
            Suite::Check => "corpus-check",
            Suite::Graphs => "corpus-graphs",
            Suite::Objects => "corpus-objects",
            Suite::JavaScript => "corpus-javascript",
        }
    }
}

/// A project to run elm on: its case, the directory to copy it from, its
/// root within, and for an app, its entries and `elm.json` per Compat.
struct Project {
    case: String,
    top: PathBuf,
    root: String,
    app: Option<apps::App>,
    admits: Option<Vec<String>>,
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
    let corpus = corpus_dir()?;
    let releases = releases(&cache_dir()?.join("nomi").join("packages"))?;
    ensure!(
        !releases.is_empty(),
        "the development cache has no releases; run `cargo xtask corpus names`"
    );
    let mut projects = Vec::new();
    if suite != Suite::Docs {
        for app in apps::load(root)?.app {
            projects.push(Project {
                case: format!("app/{}", app.name),
                top: apps::app_dir(&corpus, &app),
                root: app.root.clone(),
                app: Some(app),
                admits: None,
            });
        }
    }
    let packages = if suite == Suite::JavaScript {
        Vec::new()
    } else {
        packages::load(root)?.packages
    };
    for package in packages {
        let top = packages::package_dir(&corpus, &package);
        if !top.join("elm.json").exists() {
            continue;
        }
        let admits = (classification.package.iter())
            .find(|e| e.name == package.name && e.version == package.version)
            .map(|e| e.admits.clone());
        projects.push(Project {
            case: format!("{}@{}", package.name, package.version),
            top,
            root: ".".to_owned(),
            app: None,
            admits,
        });
    }
    projects.retain(|p| only.is_none_or(|o| p.case.contains(o)));
    let versions = compat::compats(refs)?;
    let jobs = jobs.unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()));
    let mut columns = Vec::new();
    for &version in &versions {
        let sandbox = cache_dir()?.join("sandboxes").join("corpus");
        let home = sandbox.join("home").join(version.to_string());
        elm_home(&home, version, &releases)?;
        let scratch = sandbox.join(suite.command()).join(version.to_string());
        if scratch.exists() {
            fs::remove_dir_all(&scratch)?;
        }
        let elm = elm_binary(refs, version)?;
        let cells = parallel_on(jobs, &projects, |project| {
            let admitted =
                (project.admits.as_ref()).is_none_or(|a| a.contains(&version.to_string()));
            if !admitted {
                return Ok(EXCLUDED.to_owned());
            }
            let manifest = match &project.app {
                Some(app) => compat::sandbox_manifest(&classification, app, version)?,
                None => fs::read_to_string(project.top.join("elm.json"))?,
            };
            record(suite, &elm, &home, &scratch, project, &manifest)
                .with_context(|| format!("elm {version} on {}", project.case))
        });
        let cells = cells.into_iter().collect::<Result<Vec<_>>>()?;
        eprintln!("  elm {version}: {} projects", cells.len());
        columns.push(cells);
    }
    let mut out = format!(
        "# @generated by `cargo xtask oracle {}`; regenerate, never edit.\ncase",
        suite.command()
    );
    for version in &versions {
        write!(out, "\telm {version}")?;
    }
    out.push('\n');
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
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(path, out)?;
    println!("recorded {} projects", projects.len());
    Ok(())
}

/// Every release in nomi's store under `store`: its name, version and
/// sources.
pub(crate) fn releases(store: &Path) -> Result<Vec<(String, Version, PathBuf)>> {
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
        let Ok(version) = Version::parse(version) else {
            continue;
        };
        releases.push((format!("{author}/{project}"), version, sources));
    }
    releases.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    Ok(releases)
}

/// An `ELM_HOME` in `home` for `compat` holding `releases`, kept as it is
/// when it already holds exactly those.
pub(crate) fn elm_home(
    home: &Path,
    compat: Version,
    releases: &[(String, Version, PathBuf)],
) -> Result<()> {
    let listed: Vec<(&str, Version)> = releases.iter().map(|(n, v, _)| (n.as_str(), *v)).collect();
    let registry = registry_of(compat, &listed);
    let packages_dir = home.join(compat.to_string()).join("packages");
    let registry_path = packages_dir.join("registry.dat");
    if fs::read(&registry_path).is_ok_and(|r| r == registry) {
        return Ok(());
    }
    if home.exists() {
        fs::remove_dir_all(home)?;
    }
    fs::create_dir_all(&packages_dir)?;
    for (name, version, sources) in releases {
        let to = packages_dir.join(name).join(version.to_string());
        for file in files_below(sources)? {
            let target = to.join(&file);
            fs::create_dir_all(target.parent().unwrap())?;
            fs::copy(sources.join(&file), &target)?;
        }
    }
    // Last, so a home cut short is rebuilt.
    fs::write(registry_path, registry)?;
    Ok(())
}

pub(crate) fn elm_binary(refs: &References, version: Version) -> Result<PathBuf> {
    let pin = (refs.binary.iter())
        .find(|b| b.name == "elm" && b.version == version.to_string())
        .with_context(|| format!("elm {version} is not pinned"))?;
    let elm = binary_path(pin)?;
    ensure!(
        elm.exists(),
        "{} is missing; run `cargo xtask oracle fetch`",
        elm.display()
    );
    Ok(elm)
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
) -> Result<String> {
    let id = project.case.replace('/', "__");
    let work = scratch.join("work").join(&id);
    if work.exists() {
        fs::remove_dir_all(&work)?;
    }
    copy_modules(&project.top, &work)?;
    let dir = work.join(&project.root);
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("elm.json"), manifest)?;
    let mut args: Vec<&str> = match suite {
        Suite::Docs => vec!["--docs=docs.json"],
        _ => (project.app.iter().flat_map(|a| &a.entries))
            .map(String::as_str)
            .collect(),
    };
    args.push("--report=json");
    args.push(if suite == Suite::JavaScript {
        "--output=out.js"
    } else {
        "--output=/dev/null"
    });
    let output = make(elm, home, &work, &dir, &args)?;
    if output.status.code().is_none() {
        return Ok("crash".to_owned());
    }
    let out = scratch.join("out");
    fs::create_dir_all(&out)?;
    if suite == Suite::Docs
        && let Ok(docs) = fs::read(dir.join("docs.json"))
    {
        fs::write(out.join(format!("{id}.docs.json")), &docs)?;
        fs::remove_dir_all(&work)?;
        return Ok(format!("docs {}", sha256(&docs)));
    }
    if suite == Suite::Graphs && output.status.success() {
        let stuff = dir.join("elm-stuff").join(version_dir(&dir)?);
        let mut modules = Vec::new();
        let kept = out.join(&id);
        fs::create_dir_all(&kept)?;
        for entry in fs::read_dir(&stuff)? {
            let path = entry?.path();
            let file = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if let Some(stem) = file.strip_suffix(".elmo") {
                let bytes = fs::read(&path)?;
                fs::write(kept.join(&file), &bytes)?;
                modules.push((stem.replace('-', "."), bytes));
            }
        }
        fs::remove_dir_all(&work)?;
        return Ok(graphs_cell(modules));
    }
    if suite == Suite::Objects && output.status.success() {
        let stuff = dir.join("elm-stuff").join(version_dir(&dir)?);
        let objects = fs::read(stuff.join("o.dat"))?;
        fs::write(out.join(format!("{id}.o.dat")), &objects)?;
        fs::remove_dir_all(&work)?;
        return Ok(format!("objects {}", sha256(&objects)));
    }
    if suite == Suite::JavaScript && output.status.success() {
        let mut hashes = Vec::new();
        for (mode, flag) in super::javascript::MODES {
            let output = match flag {
                None => output.clone(),
                Some(flag) => make(elm, home, &work, &dir, &[&args[..], &[flag]].concat())?,
            };
            if !output.status.success() {
                let report = String::from_utf8_lossy(&output.stderr);
                fs::write(out.join(format!("{id}.{mode}.report.json")), &*report)?;
                hashes.push("-".to_owned());
                continue;
            }
            let js = fs::read(dir.join("out.js"))?;
            fs::write(out.join(format!("{id}.{mode}.js")), &js)?;
            hashes.push(sha256(&js));
        }
        fs::remove_dir_all(&work)?;
        return Ok(format!("javascript {}", hashes.join(" ")));
    }
    if suite == Suite::Check && output.status.success() {
        fs::write(out.join(format!("{id}.stdout")), &output.stdout)?;
        fs::write(out.join(format!("{id}.stderr")), &output.stderr)?;
        fs::remove_dir_all(&work)?;
        return Ok(if output.stdout.is_empty() && output.stderr.is_empty() {
            OK.to_owned()
        } else {
            format!("ok-noisy {}", sha256(&output.stdout))
        });
    }
    let Some(json) = [&output.stderr, &output.stdout]
        .into_iter()
        .find(|o| o.starts_with(b"{"))
    else {
        // Still dying without a report: kept for a look, and skipped.
        fs::write(out.join(format!("{id}.stderr")), &output.stderr)?;
        fs::remove_dir_all(&work)?;
        return Ok("crash".to_owned());
    };
    let root = fs::canonicalize(&dir)?;
    let report = String::from_utf8(json.trim_ascii_end().to_vec())?
        .replace(root.to_str().context("a UTF-8 scratch path")?, "$ROOT");
    fs::write(out.join(format!("{id}.report.json")), &report)?;
    fs::remove_dir_all(&work)?;
    Ok(cell(&report))
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
        offline(&mut command);
        let Some(output) = timed(command, work)? else {
            // Hung, as concurrent runs very rarely leave elm; again.
            attempt += 1;
            ensure!(attempt <= RETRIES, "elm hung {RETRIES} times");
            continue;
        };
        let stderr = String::from_utf8_lossy(&output.stderr);
        let unreported = !output.status.success()
            && !output.stderr.starts_with(b"{")
            && !output.stdout.starts_with(b"{");
        // A success that says something has, so far, been one of those
        // too: alone, it is silent.
        let noisy = output.status.success() && !output.stderr.is_empty();
        let retry = stderr.contains("(file is locked)") || unreported || noisy;
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
        fs::create_dir_all(target.parent().unwrap())?;
        fs::copy(top.join(&file), &target)?;
    }
    Ok(())
}

/// Runs `command` to completion, or kills it and returns `None` once it
/// has run for [`TIMEOUT`], with its output in files in `dir`.
fn timed(mut command: Command, dir: &Path) -> Result<Option<Output>> {
    let (stdout, stderr) = (dir.join(".stdout"), dir.join(".stderr"));
    let mut child = command
        .stdin(Stdio::null())
        .stdout(fs::File::create(&stdout)?)
        .stderr(fs::File::create(&stderr)?)
        .spawn()
        .context("running elm")?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > TIMEOUT {
            child.kill()?;
            child.wait()?;
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Ok(Some(Output {
        status,
        stdout: fs::read(stdout)?,
        stderr: fs::read(stderr)?,
    }))
}

/// The `elm-stuff` directory elm made in `dir`, named after its release.
fn version_dir(dir: &Path) -> Result<String> {
    let entry = fs::read_dir(dir.join("elm-stuff"))?
        .next()
        .context("elm left no elm-stuff")??;
    Ok(entry.file_name().to_string_lossy().into_owned())
}

/// The cell for a project's module graphs, each a module's name and the
/// bytes of its `.elmo`: `graphs <digest> <count>`, the digest over each
/// name and the SHA-256 of its bytes, in the order of names.
pub(crate) fn graphs_cell(mut modules: Vec<(String, Vec<u8>)>) -> String {
    modules.sort_by(|a, b| a.0.cmp(&b.0));
    let mut listing = String::new();
    for (name, bytes) in &modules {
        listing.push_str(&format!("{name} {}\n", sha256(bytes)));
    }
    format!("graphs {} {}", sha256(listing.as_bytes()), modules.len())
}

/// The cell for a report: `errors <hash> <first title>` for one about
/// modules, `error <title>` for one about the project.
pub(crate) fn cell(report: &str) -> String {
    let value: Value = serde_json::from_str(report).unwrap_or(Value::Null);
    if value["type"] == "compile-errors" {
        let title = value["errors"][0]["problems"][0]["title"]
            .as_str()
            .unwrap_or("?");
        format!("errors {} {title}", sha256(report.as_bytes()))
    } else {
        format!("error {}", value["title"].as_str().unwrap_or("?"))
    }
}
