//! The corpus apps' test suites: each app with an
//! elm-explorations/test suite, run by elm-test 0.19.3-0 (`cargo xtask
//! oracle corpus-tests`) and by `nomi test` (`cargo xtask corpus tests`)
//! with a fixed seed and the JSON reporter, in a copy of the app outside
//! its checkout. elm-test's exit code, counts and a digest of its
//! normalized output are kept in `corpus/tests/oracle.tsv`; both outputs
//! stay in the sandbox for comparing by hand. By hand, as both download
//! from the registry: elm into the sandbox's `ELM_HOME`, nomi into the
//! development cache.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use nomi_testkit::transcript::normalize_elm_test;
use serde_json::Value;

use super::{apps, corpus_dir};
use crate::download::sha256;
use crate::oracle::cache_dir;
use crate::references::References;

const ORACLE: &str = "corpus/tests/oracle.tsv";

/// What every run is asked.
const ARGS: [&str; 6] = ["--report", "json", "--seed", "12345", "--fuzz", "100"];

/// A suite can take a while; one that takes longer than this hangs.
const LIMIT: Duration = Duration::from_secs(900);

/// A run's outcome, as the oracle keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Outcome {
    exit: Option<i32>,
    passed: String,
    failed: String,
    digest: String,
    /// Of standard error, which a suite that doesn't build or run writes.
    errors: String,
}

impl Outcome {
    fn row(&self, app: &str) -> String {
        let exit = self
            .exit
            .map_or_else(|| "killed".to_owned(), |c| c.to_string());
        format!(
            "{app}\t{exit}\t{}\t{}\t{}\t{}",
            self.passed, self.failed, self.digest, self.errors
        )
    }
}

/// The apps with a suite: their names and project directories.
fn suites(root: &Path, only: Option<&str>) -> Result<Vec<(String, PathBuf)>> {
    let corpus = corpus_dir()?;
    let mut suites = Vec::new();
    for app in apps::load(root)?.app {
        let dir = apps::app_dir(&corpus, &app).join(&app.root);
        let elm_json = fs::read_to_string(dir.join("elm.json")).unwrap_or_default();
        if dir.join("tests").is_dir()
            && elm_json.contains("\"elm-explorations/test\"")
            && only.is_none_or(|o| app.name.contains(o))
        {
            suites.push((app.name, dir));
        }
    }
    Ok(suites)
}

fn sandbox() -> Result<PathBuf> {
    Ok(cache_dir()?.join("sandboxes/corpus/corpus-tests"))
}

/// `from` copied to `to`, past `elm-stuff`, `node_modules` and `.git`,
/// and Node version pins, `package.json`'s engines among them, which would
/// have a version manager on the path fetch a Node to run elm-test with.
fn copy(from: &Path, to: &Path) -> Result<()> {
    if to.exists() {
        fs::remove_dir_all(to)?;
    }
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let skipped = [
            ".git",
            "elm-stuff",
            "node_modules",
            ".nvmrc",
            ".node-version",
            ".tool-versions",
            "package.json",
        ];
        if skipped.iter().any(|n| name == *n) {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy(&entry.path(), &to.join(&name))?;
        } else if kind.is_file() {
            fs::copy(entry.path(), to.join(&name))?;
        }
    }
    Ok(())
}

/// Runs `command` in a fresh copy of `dir` named `name`, under `who`'s
/// part of the sandbox; its outcome, its normalized output kept beside.
fn run(mut command: Command, who: &str, name: &str, dir: &Path, home: &str) -> Result<Outcome> {
    let work = sandbox()?.join("work").join(name);
    copy(dir, &work)?;
    let mut child = command
        .args(ARGS)
        .current_dir(&work)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("{name}: starting {who}"))?;
    let read = |stream: Box<dyn std::io::Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut stream = stream;
            let _ = stream.read_to_end(&mut bytes);
            bytes
        })
    };
    let stdout = read(Box::new(child.stdout.take().context("stdout")?));
    let stderr = read(Box::new(child.stderr.take().context("stderr")?));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if started.elapsed() > LIMIT {
            child.kill()?;
            child.wait()?;
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let dir_text = work.display().to_string();
    let stdout = normalize_elm_test(&stdout.join().unwrap_or_default(), &dir_text, home);
    let stderr = normalize_elm_test(&stderr.join().unwrap_or_default(), &dir_text, home);
    let kept = sandbox()?.join(who);
    fs::create_dir_all(&kept)?;
    fs::write(kept.join(format!("{name}.json")), &stdout)?;
    fs::write(kept.join(format!("{name}.stderr")), &stderr)?;
    let text = String::from_utf8_lossy(&stdout);
    let complete = (text.lines())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|e| e["event"] == "runComplete");
    let count = |field: &str| {
        (complete.as_ref())
            .and_then(|e| e[field].as_str().map(str::to_owned))
            .unwrap_or_else(|| "-".to_owned())
    };
    Ok(Outcome {
        exit: status.and_then(|s| s.code()),
        passed: count("passed"),
        failed: count("failed"),
        digest: sha256(&stdout)[..16].to_owned(),
        errors: sha256(&stderr)[..16].to_owned(),
    })
}

fn oracle(root: &Path) -> Result<BTreeMap<String, String>> {
    let text = fs::read_to_string(root.join(ORACLE))
        .with_context(|| format!("reading {ORACLE}; run `cargo xtask oracle corpus-tests`"))?;
    Ok((text.lines())
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| Some((l.split('\t').next()?.to_owned(), l.to_owned())))
        .collect())
}

/// `cargo xtask oracle corpus-tests`: elm-test's outcome on each suite.
pub(crate) fn record(root: &Path, refs: &References, only: Option<&str>) -> Result<()> {
    let elm = crate::oracle::transcripts::binary(refs, "elm", Some("0.19.3"))?;
    let runner = crate::oracle::oracles_dir()?.join("npm/node_modules/elm-test/bin/elm-test");
    let node = std::env::var_os("PATH")
        .and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join("node"))
                .find(|candidate| candidate.is_file())
        })
        .context("node isn't on PATH")?;
    let sandbox = sandbox()?;
    let bin = sandbox.join("bin");
    let home = sandbox.join("elm-home");
    fs::create_dir_all(&bin)?;
    fs::create_dir_all(&home)?;
    let link = bin.join("elm");
    if !link.exists() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(&elm, &link)?;
    }
    let path = std::env::join_paths([bin, node.parent().context("node's directory")?.to_owned()])?;
    let mut rows = if only.is_some() {
        oracle(root).unwrap_or_default()
    } else {
        BTreeMap::new()
    };
    for (name, dir) in suites(root, only)? {
        let mut command = Command::new(&node);
        command
            .arg(&runner)
            .env("PATH", &path)
            .env("ELM_HOME", &home)
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR");
        let outcome = run(
            command,
            "elm-test",
            &name,
            &dir,
            &home.display().to_string(),
        )?;
        println!("{}", outcome.row(&name));
        rows.insert(name.clone(), outcome.row(&name));
    }
    let mut text = String::from(
        "# app\texit\tpassed\tfailed\toutput\terrors: elm-test 0.19.3-0 with --report json --seed 12345 --fuzz 100 (plan 10.7)\n",
    );
    for row in rows.values() {
        text.push_str(row);
        text.push('\n');
    }
    fs::create_dir_all(root.join("corpus/tests"))?;
    fs::write(root.join(ORACLE), text)?;
    Ok(())
}

/// `cargo xtask corpus tests`: whether `nomi test` gives elm-test's
/// outcome on each suite.
pub(crate) fn check(root: &Path, only: Option<&str>) -> Result<()> {
    crate::cargo(
        root,
        &["build", "--locked", "--release", "--package", "nomi-cli"],
    )?;
    let nomi = root.join("target/release/nomi");
    let oracle = oracle(root)?;
    let cache = cache_dir()?.join("nomi");
    let mut failures = Vec::new();
    let mut passed = 0;
    for (name, dir) in suites(root, only)? {
        let Some(expected) = oracle.get(&name) else {
            failures.push(format!("{name}: not recorded"));
            continue;
        };
        let mut command = Command::new(&nomi);
        command.arg("test").env("NOMI_CACHE", &cache);
        let outcome = run(command, "nomi", &name, &dir, &cache.display().to_string())?;
        let actual = outcome.row(&name);
        if actual == *expected {
            passed += 1;
            println!("ok   {actual}");
        } else {
            println!("FAIL {actual}\n  (elm-test {expected})");
            failures.push(name);
        }
    }
    println!(
        "{passed} of {} suites as elm-test ran them",
        passed + failures.len()
    );
    if !failures.is_empty() {
        let dir = sandbox()?;
        bail!(
            "{} differ: {}; outputs in {}/{{elm-test,nomi}}",
            failures.len(),
            failures.join(", "),
            dir.display()
        );
    }
    Ok(())
}
