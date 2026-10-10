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

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use nomi_syntax::Source;
use serde::Deserialize;

use crate::corpus::compat::Version;
use crate::corpus::{corpus_dir, files_below, packages};

/// The packages every sandbox project depends on.
const DEPENDENCIES: [&str; 2] = ["elm/core", "elm/json"];

/// The module under test, beside each project's `elm.json`.
const MODULE: &str = "A.elm";

/// Runs of one check before a lock failure or a hang is an error.
const RETRIES: u32 = 5;

/// How long one elm run may take before it counts as hung. Parsing takes
/// milliseconds; building elm/core while warming up takes about a second.
const TIMEOUT: Duration = Duration::from_secs(60);

/// What a tool made of one module's syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Accepted,
    /// The title and region elm reports, or where elm-format stopped.
    Rejected(String),
    /// The tool died of an uncaught exception, which says nothing about
    /// the syntax; the ratchets skip it. elm 0.19.3 does this on some
    /// recursive type aliases.
    Crashed,
}

/// One elm release, ready to check modules of every kind.
#[derive(Debug)]
pub struct Sandbox {
    elm: PathBuf,
    home: PathBuf,
    dir: PathBuf,
}

impl Sandbox {
    /// Creates a fresh sandbox in `dir` for the `elm` binary of `compat`,
    /// and builds each project once so later runs only parse.
    pub fn create(dir: &Path, elm: &Path, compat: Version, root: &Path) -> Result<Sandbox> {
        if dir.exists() {
            fs::remove_dir_all(dir).with_context(|| format!("removing {}", dir.display()))?;
        }
        let home = dir.join("home");
        let pinned = dependencies(root)?;
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
            let (warm, _) = sandbox.check(source, b"module A exposing (a)\n\n\na =\n    1\n")?;
            ensure!(
                warm == Outcome::Accepted,
                "elm {compat} rejects a trivial module in {}: {warm:?}",
                project.display()
            );
        }
        Ok(sandbox)
    }

    /// Runs `command` to completion, or kills it and returns `None` once
    /// it has run for [`TIMEOUT`]: elm very rarely hangs.
    fn run(&self, mut command: Command) -> Result<Option<Output>> {
        let (stdout, stderr) = (self.dir.join("stdout"), self.dir.join("stderr"));
        let mut child = command
            .stdin(Stdio::null())
            .stdout(fs::File::create(&stdout)?)
            .stderr(fs::File::create(&stderr)?)
            .spawn()
            .with_context(|| format!("running {}", self.elm.display()))?;
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
            std::thread::sleep(Duration::from_millis(2));
        };
        Ok(Some(Output {
            status,
            stdout: fs::read(stdout)?,
            stderr: fs::read(stderr)?,
        }))
    }

    fn project(&self, source: Source) -> PathBuf {
        self.dir.join(match source {
            Source::Application => "application",
            Source::Package => "package",
            Source::Kernel => "kernel",
        })
    }

    /// Whether elm parses `text` as a module of a `source` project. Errors
    /// after parsing, such as unknown imports, count as accepted.
    ///
    /// When elm rejects it, also the report elm wrote, as it wrote it.
    pub fn check(&self, source: Source, text: &[u8]) -> Result<(Outcome, Option<Vec<u8>>)> {
        let project = self.project(source);
        fs::write(project.join(MODULE), text)?;
        let mut attempt = 1;
        loop {
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
            offline(&mut command);
            let Some(output) = self.run(command)? else {
                ensure!(
                    attempt < RETRIES,
                    "{} hung {RETRIES} times",
                    self.elm.display()
                );
                attempt += 1;
                continue;
            };
            let report = [&output.stderr, &output.stdout]
                .into_iter()
                .find(|o| o.starts_with(b"{"));
            let stderr = String::from_utf8_lossy(&output.stderr);
            match report {
                None if output.status.success() => return Ok((Outcome::Accepted, None)),
                // elm now and then trips over its own lock on `d.dat`.
                None if stderr.contains("(file is locked)") && attempt < RETRIES => attempt += 1,
                // GHC's report of an uncaught exception.
                None if stderr.starts_with("elm: ") && !stderr.contains("(file is locked)") => {
                    return Ok((Outcome::Crashed, None));
                }
                None => bail!(
                    "{} exited with {} and no report:\n{stderr}",
                    self.elm.display(),
                    output.status,
                ),
                Some(json) => {
                    let outcome = parse_report(json)?;
                    let report = matches!(outcome, Outcome::Rejected(_))
                        .then(|| json.trim_ascii_end().to_vec());
                    return Ok((outcome, report));
                }
            }
        }
    }
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

/// The module's syntax error, if the report has one. Upstream names a
/// module it could not parse `???`.
fn parse_report(json: &[u8]) -> Result<Outcome> {
    // elm copies tabs and carriage returns from the source into its
    // messages unescaped, which JSON forbids; only titles and regions matter.
    let json: Vec<u8> = json
        .iter()
        .map(|&b| if b < 0x20 { b' ' } else { b })
        .collect();
    let report: Report = serde_json::from_slice(&json).context("parsing elm's report")?;
    let errors = match report {
        Report::CompileErrors { errors } => errors,
        Report::Error { title } => bail!("the sandbox is broken: {title}"),
    };
    let syntax = errors.iter().find(|e| e.path == MODULE && e.name == "???");
    let Some(problem) = syntax.and_then(|e| e.problems.first()) else {
        return Ok(Outcome::Accepted);
    };
    let Region { start, end } = &problem.region;
    Ok(Outcome::Rejected(format!(
        "{} {}:{}-{}:{}",
        problem.title, start.line, start.column, end.line, end.column
    )))
}

/// Whether elm-format reads `text`, and where it stopped if not.
pub fn elm_format(binary: &Path, text: &[u8]) -> Result<Outcome> {
    let mut child = Command::new(binary)
        .args(["--stdin", "--elm-version=0.19"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("running {}", binary.display()))?;
    child.stdin.take().unwrap().write_all(text)?;
    let output = child.wait_with_output()?;
    if output.status.success() {
        return Ok(Outcome::Accepted);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let Some(at) = stderr
        .strip_prefix("Unable to parse file <STDIN>:")
        .and_then(|rest| rest.split_whitespace().next())
    else {
        bail!("elm-format exited with {}:\n{stderr}", output.status);
    };
    Ok(Outcome::Rejected(at.to_owned()))
}

/// Fills `home` as an offline `ELM_HOME` for `compat`: a registry listing
/// exactly `pinned`, and their sources in the package cache.
pub fn elm_home(home: &Path, compat: Version, pinned: &[packages::Package]) -> Result<()> {
    let packages_dir = home.join(compat.to_string()).join("packages");
    fs::create_dir_all(&packages_dir)?;
    fs::write(packages_dir.join("registry.dat"), registry(compat, pinned))?;
    let corpus = corpus_dir()?;
    for package in pinned {
        let from = packages::package_dir(&corpus, package);
        ensure!(
            from.join(packages::STAMP).exists(),
            "{} {} is not fetched; run `cargo xtask corpus fetch`",
            package.name,
            package.version
        );
        copy_tree(
            &from,
            &packages_dir.join(&package.name).join(&package.version),
        )?;
    }
    Ok(())
}

/// Points elm's proxy nowhere, so it never reaches the network and falls
/// back to the registry it has.
pub fn offline(command: &mut Command) -> &mut Command {
    command
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env_remove("NO_PROXY")
}

/// The pinned versions of [`DEPENDENCIES`].
fn dependencies(root: &Path) -> Result<Vec<packages::Package>> {
    let pinned = packages::load(root)?;
    DEPENDENCIES
        .iter()
        .map(|name| {
            pinned
                .packages
                .iter()
                .find(|p| p.name == *name)
                .cloned()
                .with_context(|| format!("{name} is not pinned in the corpus"))
        })
        .collect()
}

/// A registry listing exactly `pinned`.
fn registry(compat: Version, pinned: &[packages::Package]) -> Vec<u8> {
    let releases: Vec<(&str, Version)> = (pinned.iter())
        .map(|p| {
            (
                p.name.as_str(),
                Version::parse(&p.version).expect("a pinned version"),
            )
        })
        .collect();
    registry_of(compat, &releases)
}

/// A registry listing exactly `releases`, each package's newest version
/// first. 0.19.1 and 0.19.2 store it with Haskell's `Data.Binary`
/// (big-endian `Int64` counts); 0.19.3 switched to its own encoding
/// (little-endian, `Int64` count and `Word32` lengths). Both read the
/// packages in ascending order of author, then project.
pub fn registry_of(compat: Version, releases: &[(&str, Version)]) -> Vec<u8> {
    let legacy = compat < Version::parse("0.19.3").expect("a version");
    let mut packages: std::collections::BTreeMap<(&str, &str), Vec<Version>> =
        std::collections::BTreeMap::new();
    for (name, version) in releases {
        let (author, project) = name.split_once('/').expect("author/project");
        packages
            .entry((author, project))
            .or_default()
            .push(*version);
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
    for ((author, project), mut versions) in packages {
        for part in [author, project] {
            out.push(part.len() as u8);
            out.extend(part.as_bytes());
        }
        versions.sort_by(|a, b| b.cmp(a));
        versions.dedup();
        out.extend(versions[0].parts().map(|n| n as u8));
        length(&mut out, versions.len() - 1);
        for version in &versions[1..] {
            out.extend(version.parts().map(|n| n as u8));
        }
    }
    out
}

fn manifest(source: Source, compat: Version, pinned: &[packages::Package]) -> String {
    let exact = |p: &packages::Package| format!("\"{}\": \"{}\"", p.name, p.version);
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

/// Copies a fetched package, leaving out the corpus's stamp.
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    for file in files_below(from)? {
        if file == packages::STAMP {
            continue;
        }
        let target = to.join(&file);
        fs::create_dir_all(target.parent().unwrap())?;
        fs::copy(from.join(&file), &target)
            .with_context(|| format!("copying {}", from.join(&file).display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pinned() -> Vec<packages::Package> {
        [("elm/core", "1.0.5"), ("elm/json", "1.1.4")]
            .map(|(name, version)| packages::Package {
                name: name.to_owned(),
                version: version.to_owned(),
                sha1: String::new(),
            })
            .to_vec()
    }

    #[test]
    fn legacy_registry_is_big_endian_binary() {
        let bytes = registry(Version::parse("0.19.1").unwrap(), &pinned());
        let mut expected = vec![0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 2];
        expected.extend(b"\x03elm\x04core\x01\x00\x05\0\0\0\0\0\0\0\0");
        expected.extend(b"\x03elm\x04json\x01\x01\x04\0\0\0\0\0\0\0\0");
        assert_eq!(bytes, expected);
    }

    #[test]
    fn registry_from_0_19_3_is_little_endian() {
        let bytes = registry(Version::parse("0.19.3").unwrap(), &pinned());
        let mut expected = vec![2, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0];
        expected.extend(b"\x03elm\x04core\x01\x00\x05\0\0\0\0");
        expected.extend(b"\x03elm\x04json\x01\x01\x04\0\0\0\0");
        assert_eq!(bytes, expected);
    }

    #[test]
    fn manifests_are_json() {
        let compat = Version::parse("0.19.2").unwrap();
        for source in [Source::Application, Source::Package, Source::Kernel] {
            let text = manifest(source, compat, &pinned());
            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert!(json["type"].is_string(), "{text}");
        }
    }

    #[test]
    fn raw_control_characters_are_tolerated() {
        let report = b"{\"type\":\"compile-errors\",\"errors\":[{\"path\":\"A.elm\",\"name\":\"???\",\"problems\":[{\"title\":\"TAB\",\"region\":{\"start\":{\"line\":1,\"column\":2},\"end\":{\"line\":1,\"column\":3}},\"message\":[\"\t\r\"]}]}]}";
        assert_eq!(
            parse_report(report).unwrap(),
            Outcome::Rejected("TAB 1:2-1:3".to_owned())
        );
    }

    #[test]
    fn syntax_errors_belong_to_the_unnamed_module() {
        let report = br#"{"type":"compile-errors","errors":[{"path":"A.elm","name":"???","problems":[{"title":"UNFINISHED LIST","region":{"start":{"line":3,"column":5},"end":{"line":3,"column":5}},"message":[]}]}]}"#;
        assert_eq!(
            parse_report(report).unwrap(),
            Outcome::Rejected("UNFINISHED LIST 3:5-3:5".to_owned())
        );
        let imports = br#"{"type":"compile-errors","errors":[{"path":"A.elm","name":"A","problems":[{"title":"MODULE NOT FOUND","region":{"start":{"line":2,"column":8},"end":{"line":2,"column":15}},"message":[]}]}]}"#;
        assert_eq!(parse_report(imports).unwrap(), Outcome::Accepted);
        let broken = br#"{"type":"error","path":"elm.json","title":"BAD JSON","message":[]}"#;
        assert!(parse_report(broken).is_err());
    }
}
