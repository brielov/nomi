//! Offline sandboxes for running a pinned `elm` on one module at a time.
//!
//! Each sandbox has its own `ELM_HOME`, holding a synthesized
//! `registry.dat` and copies of the corpus's elm/core and elm/json, and one
//! project for each kind of source: an application naming the Compat, a
//! package and a kernel package. The module under test sits beside the
//! project's `elm.json`, outside its source directories, so elm accepts any
//! module name for it and reports a syntax error under the name `???`
//! (`Build.hs`, `crawlRoot`). The proxy points nowhere, so elm never
//! reaches the network and falls back to the registry it has.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use nomi_base::Compat;
use nomi_project::Version;
use nomi_syntax::Source;
use serde::Deserialize;

use super::process::run_to_files;
use crate::corpus::{corpus_dir, packages};

/// The packages every sandbox project depends on.
const DEPENDENCIES: [&str; 2] = ["elm/core", "elm/json"];

/// The module under test, beside each project's `elm.json`.
const MODULE: &str = "A.elm";

/// Runs of one elm before a run that failed as runs sharing an
/// `ELM_HOME` sometimes fail, or that hung, is kept as it went.
pub(crate) const RETRIES: u32 = 5;

/// How long one elm run may take before it counts as hung. Parsing takes
/// milliseconds; building elm/core while warming up takes about a second.
const TIMEOUT: Duration = Duration::from_secs(60);

/// What elm made of one module's syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    Accepted,
    /// The title and region elm reports, and its report as it wrote it.
    Rejected {
        summary: String,
        report: Vec<u8>,
    },
    /// elm died of an uncaught exception, which says nothing about the
    /// syntax; the ratchets skip it. elm 0.19.3 does this on some
    /// recursive type aliases.
    Crashed,
}

/// One elm release, ready to check modules of every kind.
#[derive(Debug)]
pub(crate) struct Sandbox {
    elm: PathBuf,
    home: PathBuf,
    dir: PathBuf,
}

impl Sandbox {
    /// Creates a fresh sandbox in `dir` for the `elm` binary of `compat`,
    /// and builds each project once so later runs only parse.
    pub(crate) fn create(dir: &Path, elm: &Path, compat: Compat, root: &Path) -> Result<Sandbox> {
        super::remove_tree(dir)?;
        let home = dir.join("home");
        let pinned = corpus_releases(root, &DEPENDENCIES)?;
        elm_home(&home, compat, &pinned)?;
        let sandbox = Sandbox {
            elm: elm.to_path_buf(),
            home,
            dir: dir.to_path_buf(),
        };
        for source in [Source::Application, Source::Package, Source::Kernel] {
            let project = sandbox.project(source);
            fs::create_dir_all(project.join("src"))?;
            fs::write(project.join("elm.json"), manifest(source, compat, &pinned))?;
            let warm = sandbox.check(source, b"module A exposing (a)\n\n\na =\n    1\n")?;
            ensure!(
                warm == Verdict::Accepted,
                "elm {compat} rejects a trivial module in {}: {warm:?}",
                project.display()
            );
        }
        Ok(sandbox)
    }

    fn project(&self, source: Source) -> PathBuf {
        self.dir.join(match source {
            Source::Application => "application",
            Source::Package => "package",
            Source::Kernel => "kernel",
        })
    }

    /// Whether elm parses `text` as a module of a `source` project. Errors
    /// after parsing, such as unknown imports, count as accepted. A run
    /// that hung, or tripped over a lock, [`RETRIES`] times is an error.
    pub(crate) fn check(&self, source: Source, text: &[u8]) -> Result<Verdict> {
        let project = self.project(source);
        fs::write(project.join(MODULE), text)?;
        let output = retrying(RETRIES, |_, _| {
            let mut command = Command::new(&self.elm);
            command
                // One RTS thread: the oracle already runs a process per core.
                .args([
                    "make",
                    MODULE,
                    "--output=/dev/null",
                    "--report=json",
                    "+RTS",
                    "-N1",
                    "-RTS",
                ])
                .current_dir(&project)
                .env("ELM_HOME", &self.home);
            // elm very rarely hangs, and now and then fails without a
            // report, tripping over its own lock on `d.dat`.
            let ran = run_to_files(offline(&mut command), &self.dir, TIMEOUT)?;
            let again = ran.as_ref().is_none_or(|output| {
                !output.status.success() && json_report(output).is_none() && locked(&output.stderr)
            });
            Ok(if again {
                Retry::Again(ran)
            } else {
                Retry::Done(ran)
            })
        })?;
        let Some(output) = output else {
            bail!("{} hung {RETRIES} times", self.elm.display());
        };
        let stderr = String::from_utf8_lossy(&output.stderr);
        match json_report(&output) {
            None if output.status.success() => Ok(Verdict::Accepted),
            // GHC's report of an uncaught exception.
            None if stderr.starts_with("elm: ") && !locked(&output.stderr) => Ok(Verdict::Crashed),
            // Death by a signal, as 0.19.3's on some recursive aliases,
            // which then dies of a stack overflow or of SIGSEGV.
            None if output.status.code().is_none() => Ok(Verdict::Crashed),
            None => bail!(
                "{} exited with {} and no report:\n{stderr}",
                self.elm.display(),
                output.status,
            ),
            Some(json) => Ok(match syntax_error(json)? {
                Some(summary) => Verdict::Rejected {
                    summary,
                    report: json.trim_ascii_end().to_vec(),
                },
                None => Verdict::Accepted,
            }),
        }
    }
}

/// elm's `--report=json`, on whichever stream it wrote it.
pub(crate) fn json_report(output: &Output) -> Option<&[u8]> {
    [&output.stderr, &output.stdout]
        .into_iter()
        .find(|o| o.starts_with(b"{"))
        .map(Vec::as_slice)
}

/// `json` with each control character a space: elm copies tabs and
/// carriage returns from the source into its messages unescaped, which
/// JSON forbids, while the newlines it writes are escaped.
pub(crate) fn sanitized(json: &[u8]) -> Vec<u8> {
    json.iter()
        .map(|&b| if b < 0x20 { b' ' } else { b })
        .collect()
}

/// The first title of elm's report: the report's own, or its first
/// problem's; `?` without either.
pub(crate) fn first_title(report: &serde_json::Value) -> &str {
    (report["title"].as_str())
        .or_else(|| report["errors"][0]["problems"][0]["title"].as_str())
        .unwrap_or("?")
}

/// Each module's graph elm left in `stuff`, its `elm-stuff/<release>`:
/// the module's name and its `.elmo` file's.
pub(crate) fn elmo_files(stuff: &Path) -> Result<Vec<(String, String)>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(stuff)? {
        let file = entry?.file_name().to_string_lossy().into_owned();
        if let Some(stem) = file.strip_suffix(".elmo") {
            files.push((stem.replace('-', "."), file));
        }
    }
    Ok(files)
}

/// What one run of a retried command came to.
pub(crate) enum Retry<T> {
    /// Kept.
    Done(T),
    /// Run again, unless it was the last run allowed; then kept.
    Again(T),
}

/// The first of `run`'s results that is [`Retry::Done`], or the last of
/// `limit` runs. `run` is given each run's number, from 1, and the
/// result of the one before it if there was one.
pub(crate) fn retrying<T>(
    limit: u32,
    mut run: impl FnMut(u32, Option<T>) -> Result<Retry<T>>,
) -> Result<T> {
    let (mut attempt, mut previous) = (1, None);
    loop {
        match run(attempt, previous.take())? {
            Retry::Again(ran) if attempt < limit => previous = Some(ran),
            Retry::Done(ran) | Retry::Again(ran) => return Ok(ran),
        }
        attempt += 1;
    }
}

/// Whether elm tripped over its own lock on a file, as it now and then
/// does where runs share an `ELM_HOME`.
pub(crate) fn locked(stderr: &[u8]) -> bool {
    String::from_utf8_lossy(stderr).contains("(file is locked)")
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum Report {
    CompileErrors { errors: Vec<ModuleErrors> },
    Error { title: String },
}

#[derive(Deserialize)]
struct ModuleErrors {
    path: String,
    name: String,
    problems: Vec<Problem>,
}

#[derive(Deserialize)]
struct Problem {
    title: String,
    region: Region,
}

#[derive(Deserialize)]
struct Region {
    start: Position,
    end: Position,
}

#[derive(Deserialize)]
struct Position {
    line: u32,
    column: u32,
}

/// The module's syntax error, as its title and region, if the report
/// has one. Upstream names a module it could not parse `???`.
fn syntax_error(json: &[u8]) -> Result<Option<String>> {
    // Only titles and regions matter.
    let report: Report =
        serde_json::from_slice(&sanitized(json)).context("parsing elm's report")?;
    let errors = match report {
        Report::CompileErrors { errors } => errors,
        Report::Error { title } => bail!("the sandbox is broken: {title}"),
    };
    let syntax = errors.iter().find(|e| e.path == MODULE && e.name == "???");
    Ok(syntax.and_then(|e| e.problems.first()).map(|problem| {
        let Region { start, end } = &problem.region;
        format!(
            "{} {}:{}-{}:{}",
            problem.title, start.line, start.column, end.line, end.column
        )
    }))
}

/// A package release an `ELM_HOME` holds.
#[derive(Debug, Clone)]
pub(crate) struct Release {
    /// `<author>/<project>`.
    pub name: String,
    pub version: Version,
    /// Its files.
    pub dir: PathBuf,
}

impl Release {
    fn new(name: String, version: &str, dir: PathBuf) -> Result<Release> {
        let version = (version.parse().ok())
            .with_context(|| format!("{name} has a version that isn't one: {version}"))?;
        Ok(Release { name, version, dir })
    }
}

/// The corpus's pins of the packages `names`, each fetched.
pub(crate) fn corpus_releases(root: &Path, names: &[&str]) -> Result<Vec<Release>> {
    let pinned = fetched(root, |name| names.contains(&name))?;
    ensure!(
        pinned.len() == names.len(),
        "the corpus doesn't pin {names:?}"
    );
    Ok(pinned)
}

/// The corpus's pins of the packages `wanted` takes, by name, each
/// fetched.
pub(crate) fn fetched(root: &Path, wanted: impl Fn(&str) -> bool) -> Result<Vec<Release>> {
    let corpus = corpus_dir()?;
    (packages::load(root)?.packages.into_iter())
        .filter(|p| wanted(&p.name))
        .map(|package| {
            let dir = packages::package_dir(&corpus, &package);
            ensure!(
                dir.join(packages::STAMP).exists(),
                "{} {} is not fetched; run `cargo xtask corpus fetch`",
                package.name,
                package.version
            );
            Release::new(package.name, &package.version, dir)
        })
        .collect()
}

/// The fixture packages the tests give nomi, under
/// `crates/compiler/tests/packages`.
pub(crate) fn fixture_releases() -> Result<Vec<Release>> {
    (nomi_testkit::packages::manifest().into_iter())
        .map(|package| {
            let dir = package.dir();
            Release::new(package.name, &package.version, dir)
        })
        .collect()
}

/// Fills `home` as an offline `ELM_HOME` for `compat`: a registry listing
/// exactly `releases`, and their files in the package cache, but the
/// corpus's stamps. A home that already lists exactly them is kept as it
/// is, with whatever elm built in it. Gives the package cache.
pub(crate) fn elm_home(home: &Path, compat: Compat, releases: &[Release]) -> Result<PathBuf> {
    let registry = registry_of(compat, releases);
    let packages = home.join(compat.as_str()).join("packages");
    let registry_path = packages.join("registry.dat");
    if fs::read(&registry_path).is_ok_and(|r| r == registry) {
        return Ok(packages);
    }
    super::remove_tree(home)?;
    fs::create_dir_all(&packages)?;
    for release in releases {
        let to = packages
            .join(&release.name)
            .join(release.version.to_string());
        nomi_testkit::copy_tree(&release.dir, &to, |p| p != Path::new(packages::STAMP))
            .with_context(|| format!("copying {}", release.dir.display()))?;
    }
    // Last, so a home cut short is rebuilt.
    fs::write(registry_path, registry)?;
    Ok(packages)
}

/// Points elm's proxy nowhere, so it never reaches the network and falls
/// back to the registry it has.
pub(crate) fn offline(command: &mut Command) -> &mut Command {
    command
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env_remove("NO_PROXY")
}

/// A registry listing exactly `releases`, each package's newest version
/// first. 0.19.1 and 0.19.2 store it with Haskell's `Data.Binary`
/// (big-endian `Int64` counts); 0.19.3 switched to its own encoding
/// (little-endian, `Int64` count and `Word32` lengths). Both read the
/// packages in ascending order of author, then project.
fn registry_of(compat: Compat, releases: &[Release]) -> Vec<u8> {
    let legacy = match compat {
        Compat::V0_19_1 | Compat::V0_19_2 => true,
        Compat::V0_19_3 => false,
    };
    let mut packages: BTreeMap<(&str, &str), Vec<Version>> = BTreeMap::new();
    for release in releases {
        let (author, project) = release.name.split_once('/').expect("author/project");
        packages
            .entry((author, project))
            .or_default()
            .push(release.version);
    }
    let count = releases.len() as u64;
    let mut out = Vec::new();
    let length = |out: &mut Vec<u8>, n: usize| {
        if legacy {
            out.extend((n as u64).to_be_bytes());
        } else {
            out.extend((n as u32).to_le_bytes());
        }
    };
    if legacy {
        out.extend(count.to_be_bytes());
    } else {
        out.extend(count.to_le_bytes());
    }
    length(&mut out, packages.len());
    let numbers = |v: &Version| [v.major, v.minor, v.patch].map(|n| n as u8);
    for ((author, project), mut versions) in packages {
        for part in [author, project] {
            out.push(part.len() as u8);
            out.extend(part.as_bytes());
        }
        versions.sort_by(|a, b| b.cmp(a));
        versions.dedup();
        out.extend(numbers(&versions[0]));
        length(&mut out, versions.len() - 1);
        for version in &versions[1..] {
            out.extend(numbers(version));
        }
    }
    out
}

fn manifest(source: Source, compat: Compat, pinned: &[Release]) -> String {
    let exact = |p: &Release| format!("\"{}\": \"{}\"", p.name, p.version);
    match source {
        Source::Application => {
            let direct: Vec<String> = pinned.iter().map(exact).collect();
            format!(
                r#"{{"type": "application", "source-directories": ["src"], "elm-version": "{compat}", "dependencies": {{"direct": {{{}}}, "indirect": {{}}}}, "test-dependencies": {{"direct": {{}}, "indirect": {{}}}}}}"#,
                direct.join(", ")
            )
        }
        Source::Package | Source::Kernel => {
            // Upstream lets a package declare effects and infix operators
            // only under `elm/` and `elm-explorations/`.
            let name = if source == Source::Kernel {
                "elm/sandbox"
            } else {
                "author/sandbox"
            };
            format!(
                r#"{{"type": "package", "name": "{name}", "summary": "sandbox", "license": "BSD-3-Clause", "version": "1.0.0", "exposed-modules": [], "elm-version": "0.19.0 <= v < 0.20.0", "dependencies": {{"elm/core": "1.0.0 <= v < 2.0.0"}}, "test-dependencies": {{}}}}"#
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pinned() -> Vec<Release> {
        [("elm/core", "1.0.5"), ("elm/json", "1.1.4")]
            .map(|(name, version)| Release {
                name: name.to_owned(),
                version: version.parse().unwrap(),
                dir: PathBuf::new(),
            })
            .to_vec()
    }

    #[test]
    fn legacy_registry_is_big_endian_binary() {
        let bytes = registry_of(Compat::V0_19_1, &pinned());
        let mut expected = vec![0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 2];
        expected.extend(b"\x03elm\x04core\x01\x00\x05\0\0\0\0\0\0\0\0");
        expected.extend(b"\x03elm\x04json\x01\x01\x04\0\0\0\0\0\0\0\0");
        assert_eq!(bytes, expected);
    }

    #[test]
    fn registry_from_0_19_3_is_little_endian() {
        let bytes = registry_of(Compat::V0_19_3, &pinned());
        let mut expected = vec![2, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0];
        expected.extend(b"\x03elm\x04core\x01\x00\x05\0\0\0\0");
        expected.extend(b"\x03elm\x04json\x01\x01\x04\0\0\0\0");
        assert_eq!(bytes, expected);
    }

    #[test]
    fn manifests_are_json() {
        for source in [Source::Application, Source::Package, Source::Kernel] {
            let text = manifest(source, Compat::V0_19_2, &pinned());
            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert!(json["type"].is_string(), "{text}");
        }
    }

    #[test]
    fn raw_control_characters_are_tolerated() {
        let report = b"{\"type\":\"compile-errors\",\"errors\":[{\"path\":\"A.elm\",\"name\":\"???\",\"problems\":[{\"title\":\"TAB\",\"region\":{\"start\":{\"line\":1,\"column\":2},\"end\":{\"line\":1,\"column\":3}},\"message\":[\"\t\r\"]}]}]}";
        assert_eq!(
            syntax_error(report).unwrap().as_deref(),
            Some("TAB 1:2-1:3")
        );
    }

    #[test]
    fn syntax_errors_belong_to_the_unnamed_module() {
        let report = br#"{"type":"compile-errors","errors":[{"path":"A.elm","name":"???","problems":[{"title":"UNFINISHED LIST","region":{"start":{"line":3,"column":5},"end":{"line":3,"column":5}},"message":[]}]}]}"#;
        assert_eq!(
            syntax_error(report).unwrap().as_deref(),
            Some("UNFINISHED LIST 3:5-3:5")
        );
        let imports = br#"{"type":"compile-errors","errors":[{"path":"A.elm","name":"A","problems":[{"title":"MODULE NOT FOUND","region":{"start":{"line":2,"column":8},"end":{"line":2,"column":15}},"message":[]}]}]}"#;
        assert_eq!(syntax_error(imports).unwrap(), None);
        let broken = br#"{"type":"error","path":"elm.json","title":"BAD JSON","message":[]}"#;
        assert!(syntax_error(broken).is_err());
    }
}
