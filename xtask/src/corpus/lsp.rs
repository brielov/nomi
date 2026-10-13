//! The corpus differential suite of the language server (plan 9.20a): a
//! session for each project, generated from its own code, recorded from
//! elm-language-server by `cargo xtask oracle corpus-lsp` and replayed
//! against nomi by `cargo xtask corpus lsp`.
//!
//! The projects are the apps whose `elm.json` elm accepts and a sample of
//! the packages, each as 0.19.1, which elm-language-server is built
//! around. A session opens up to three modules, asks for their symbols and
//! folding, then hover, definition and references at up to eight names in
//! each, and completion at two. Everything is chosen by hashing the seed
//! with the project and the module texts, so recording and replay make the
//! same session, and a module's change changes only its own requests.
//!
//! `corpus/lsp/oracle.tsv` holds each answer's digest; the transcripts
//! stay in the sandbox, elm-language-server's and nomi's, for triage.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use lsp_server::Connection;
use lsp_types::Url;
use nomi_engine::Store;
use nomi_engine::index::Memo;
use nomi_testkit::lsp::{self, Driver, Session, Step, Timing};
use nomi_testkit::ratchet::Suite;
use serde_json::{Value, json};

use super::compat::{self, Version};
use super::{apps, corpus_dir, packages};
use crate::download::sha256;
use crate::oracle::cache_dir;
use crate::references::References;

const DIR: &str = "corpus/lsp";
const ORACLE: &str = "corpus/lsp/oracle.tsv";
/// The seed every choice hashes with; changing it changes every session.
const SEED: &str = "nomi corpus-lsp 1";
/// How many packages join the apps.
const PACKAGES: usize = 100;
const MODULES: usize = 3;
const NAMES: usize = 8;
const COMPLETIONS: usize = 2;

/// A project of the suite: its case, its directory, its `elm.json` as
/// 0.19.1 reads it, and its session.
pub(super) struct Project {
    case: String,
    root: PathBuf,
    manifest: String,
    session: Session,
}

/// A number from the seed and `parts`, for choosing.
fn pick(parts: &[&str]) -> u64 {
    let mut text = String::from(SEED);
    for part in parts {
        text.push('\u{0}');
        text.push_str(part);
    }
    u64::from_str_radix(&sha256(text.as_bytes())[..16], 16).unwrap()
}

/// The suite's projects, each with its session.
fn projects(root: &Path) -> Result<Vec<Project>> {
    let classification = compat::load(root)?;
    let corpus = corpus_dir()?;
    let v = Version::parse("0.19.1")?;
    let check = check_cells(root)?;
    let accepted = |case: &str| check.get(case).is_some_and(|cell| cell == "ok");
    let mut found = Vec::new();
    for app in apps::load(root)?.app {
        let case = format!("app/{}", app.name);
        if !accepted(&case) {
            continue;
        }
        let manifest = compat::sandbox_manifest(&classification, &app, v)?;
        // A source directory outside the app's root would have to come
        // along outside the session's copy too, which the recorder's copy
        // leaves behind; inside, it's inside `.` as well.
        let elm: Value = serde_json::from_str(&manifest)?;
        let outside = (elm["source-directories"].as_array().into_iter().flatten())
            .any(|d| d.as_str().is_some_and(|d| d.starts_with("..")));
        if outside {
            continue;
        }
        let dir = apps::app_dir(&corpus, &app).join(&app.root);
        found.push((case, dir, manifest));
    }
    let mut candidates: Vec<(u64, String, PathBuf)> = Vec::new();
    for package in packages::load(root)?.packages {
        let case = format!("{}@{}", package.name, package.version);
        let excluded = (classification.package.iter())
            .find(|p| p.name == package.name && p.version == package.version)
            .is_some_and(|p| !p.admits.iter().any(|a| a == "0.19.1"));
        if excluded || !accepted(&case) {
            continue;
        }
        candidates.push((
            pick(&[&case]),
            case,
            packages::package_dir(&corpus, &package),
        ));
    }
    candidates.sort();
    for (_, case, dir) in candidates.into_iter().take(PACKAGES) {
        let manifest = fs::read_to_string(dir.join("elm.json"))?;
        found.push((case, dir, manifest));
    }
    let mut projects: Vec<Project> = found
        .into_iter()
        .map(|(case, root, manifest)| {
            let session = session(&case, &root, &manifest)?;
            Ok(Project {
                case,
                root,
                manifest,
                session,
            })
        })
        .collect::<Result<_>>()?;
    // An editor's trace of an app is one more project, the app's again.
    let mut traces = Vec::new();
    for project in &projects {
        let Some(app) = project.case.strip_prefix("app/") else {
            continue;
        };
        let path = root.join(super::trace::DIR).join(format!("{app}.json"));
        if let Ok(text) = fs::read_to_string(&path) {
            traces.push(Project {
                case: format!("trace/{app}"),
                root: project.root.clone(),
                manifest: project.manifest.clone(),
                session: serde_json::from_str(&text)
                    .with_context(|| format!("{}", path.display()))?,
            });
        }
    }
    projects.extend(traces);
    Ok(projects)
}

/// `corpus/check/oracle.tsv`'s 0.19.1 cell for each case.
fn check_cells(root: &Path) -> Result<BTreeMap<String, String>> {
    let text = fs::read_to_string(root.join("corpus/check/oracle.tsv"))?;
    Ok(text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("case\t"))
        .filter_map(|l| {
            let mut cells = l.split('\t');
            Some((cells.next()?.to_owned(), cells.next()?.to_owned()))
        })
        .collect())
}

/// The modules of a project's source directories, by path from its root.
fn modules(root: &Path, manifest: &str) -> Vec<String> {
    let elm: Value = serde_json::from_str(manifest).unwrap_or_default();
    let dirs: Vec<String> = match elm["type"].as_str() {
        Some("application") => (elm["source-directories"].as_array().into_iter().flatten())
            .filter_map(|d| d.as_str().map(str::to_owned))
            .collect(),
        _ => vec!["src".to_owned()],
    };
    let mut found = Vec::new();
    for dir in dirs {
        let mut pending = vec![root.join(&dir)];
        while let Some(at) = pending.pop() {
            let Ok(entries) = fs::read_dir(&at) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|e| e == "elm")
                    && let Ok(relative) = path.strip_prefix(root)
                    && !relative.starts_with("..")
                {
                    found.push(relative.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// The positions of names in `text` outside comments and strings, as
/// LSP positions in UTF-16 columns.
fn names(text: &str) -> Vec<(u32, u32)> {
    let mut found = Vec::new();
    let mut line = 0u32;
    let mut column = 0u32;
    let mut chars = text.chars().peekable();
    let mut previous = ' ';
    while let Some(c) = chars.next() {
        let start = (line, column);
        let width = c.len_utf16() as u32;
        let advance = |c: char, line: &mut u32, column: &mut u32| {
            if c == '\n' {
                *line += 1;
                *column = 0;
            } else {
                *column += c.len_utf16() as u32;
            }
        };
        match c {
            '-' if chars.peek() == Some(&'-') => {
                // A line comment, to the line's end.
                advance(c, &mut line, &mut column);
                while let Some(&n) = chars.peek() {
                    if n == '\n' {
                        break;
                    }
                    advance(n, &mut line, &mut column);
                    chars.next();
                }
            }
            '{' if chars.peek() == Some(&'-') => {
                advance(c, &mut line, &mut column);
                // Its `-` opens it, and nests nothing.
                advance('-', &mut line, &mut column);
                chars.next();
                let mut depth = 1;
                let mut last = ' ';
                for n in chars.by_ref() {
                    advance(n, &mut line, &mut column);
                    match (last, n) {
                        ('{', '-') => depth += 1,
                        ('-', '}') => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    last = n;
                }
            }
            '"' if text_starts(&chars, "\"\"") => {
                // A multiline string, to its `\"\"\"`.
                advance(c, &mut line, &mut column);
                for _ in 0..2 {
                    advance('"', &mut line, &mut column);
                    chars.next();
                }
                let mut quotes = 0;
                for n in chars.by_ref() {
                    advance(n, &mut line, &mut column);
                    quotes = if n == '"' { quotes + 1 } else { 0 };
                    if quotes == 3 {
                        break;
                    }
                }
            }
            '"' | '\'' => {
                advance(c, &mut line, &mut column);
                let mut escaped = false;
                for n in chars.by_ref() {
                    advance(n, &mut line, &mut column);
                    if escaped {
                        escaped = false;
                    } else if n == '\\' {
                        escaped = true;
                    } else if n == c {
                        break;
                    }
                }
            }
            c if c.is_alphabetic() && !previous.is_alphanumeric() && previous != '_' => {
                found.push(start);
                column += width;
                while let Some(&n) = chars.peek() {
                    if n.is_alphanumeric() || n == '_' {
                        advance(n, &mut line, &mut column);
                        chars.next();
                    } else {
                        break;
                    }
                }
                previous = 'a';
                continue;
            }
            c => advance(c, &mut line, &mut column),
        }
        previous = c;
    }
    found
}

/// Whether what `chars` has next starts with `prefix`.
fn text_starts(chars: &std::iter::Peekable<std::str::Chars<'_>>, prefix: &str) -> bool {
    chars
        .clone()
        .take(prefix.chars().count())
        .eq(prefix.chars())
}

fn step(method: &str, file: &str, at: Option<(u32, u32)>) -> Result<Step> {
    let mut step = json!({ "method": method, "file": file });
    if let Some((line, character)) = at {
        step["at"] = json!({ "line": line, "character": character });
    }
    // As VS Code asks.
    if method == "textDocument/references" {
        step["params"] = json!({ "context": { "includeDeclaration": true } });
    }
    Ok(serde_json::from_value(step)?)
}

/// The session of a project.
fn session(case: &str, root: &Path, manifest: &str) -> Result<Session> {
    let mut modules: Vec<(u64, String)> = (modules(root, manifest).into_iter())
        .map(|m| (pick(&[case, &m]), m))
        .collect();
    modules.sort();
    let opened: Vec<String> = modules.into_iter().take(MODULES).map(|(_, m)| m).collect();
    let mut requests = Vec::new();
    for module in &opened {
        let text = fs::read_to_string(root.join(module))?;
        requests.push(step("textDocument/documentSymbol", module, None)?);
        requests.push(step("textDocument/foldingRange", module, None)?);
        let mut at: Vec<(u64, (u32, u32))> = (names(&text).into_iter())
            .map(|p| (pick(&[case, module, &format!("{}:{}", p.0, p.1)]), p))
            .collect();
        at.sort();
        for (_, p) in at.iter().take(NAMES) {
            for method in [
                "textDocument/hover",
                "textDocument/definition",
                "textDocument/references",
            ] {
                requests.push(step(method, module, Some(*p))?);
            }
        }
        for (_, (line, character)) in at.iter().skip(NAMES).take(COMPLETIONS) {
            // Just after the name's first character, as one types it.
            requests.push(step(
                "textDocument/completion",
                module,
                Some((*line, character + 1)),
            )?);
        }
    }
    Ok(Session {
        open: opened,
        requests,
        ..Session::default()
    })
}

/// A project copied for a session: its root, less what tools write and
/// Node version pins, with its `elm.json` as 0.19.1 reads it.
pub(super) fn copy(project: &Project, to: &Path) -> Result<()> {
    if to.exists() {
        fs::remove_dir_all(to)?;
    }
    copy_tree(&project.root, to)?;
    fs::write(to.join("elm.json"), &project.manifest)?;
    Ok(())
}

/// `from` copied to `to`, less what tools write and Node version pins.
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    let mut pending = vec![(from.to_owned(), to.to_owned())];
    while let Some((from, to)) = pending.pop() {
        fs::create_dir_all(&to)?;
        for entry in fs::read_dir(&from)? {
            let entry = entry?;
            let name = entry.file_name();
            // Node version pins, `package.json`'s engines among them, would
            // have a version manager on the path fetch a Node to run
            // elm-language-server with.
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
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                pending.push((entry.path(), to.join(&name)));
            } else if file_type.is_file() {
                fs::copy(entry.path(), to.join(&name))?;
            }
        }
    }
    Ok(())
}

/// Each answer's digest, the diagnostics first, as the oracle stores them.
fn digests(transcript: &Value) -> Vec<(String, String)> {
    let mut found = vec![(
        "diagnostics".to_owned(),
        sha256(lsp::to_text(&transcript["diagnostics"]).as_bytes()),
    )];
    for (i, request) in transcript["requests"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let method = request["method"].as_str().unwrap_or_default();
        found.push((
            format!("{i:02} {method}"),
            sha256(lsp::to_text(request).as_bytes()),
        ));
    }
    found
}

/// A project's name in the sandbox. Without an `@`: elm-language-server
/// writes it in URIs as `%40` in some places and not others, and then
/// fails to find the package's own files.
fn sandbox_name(case: &str) -> String {
    case.replace('/', "__").replace('@', "-")
}

pub(super) fn sandbox() -> Result<PathBuf> {
    Ok(cache_dir()?.join("sandboxes/corpus/corpus-lsp"))
}

/// The `ELM_HOME` elm-language-server runs with, and its package cache.
pub(super) fn home_for_lsp() -> Result<(PathBuf, PathBuf)> {
    home(Version::parse("0.19.1")?)
}

/// The project of `case`, with its session.
pub(super) fn project(root: &Path, case: &str) -> Result<Project> {
    (projects(root)?.into_iter())
        .find(|p| p.case == case)
        .with_context(|| format!("{case} isn't one of the suite's projects"))
}

/// The `ELM_HOME` the corpus suites share, holding every release the
/// development cache has, and its package cache.
fn home(compat: Version) -> Result<(PathBuf, PathBuf)> {
    let home = cache_dir()?
        .join("sandboxes/corpus/home")
        .join(compat.to_string());
    let releases = crate::oracle::corpus_runs::releases(&cache_dir()?.join("nomi/packages"))?;
    ensure!(
        !releases.is_empty(),
        "the development cache has no releases; run `cargo xtask corpus names`"
    );
    crate::oracle::corpus_runs::elm_home(&home, compat, &releases)?;
    let packages = fs::canonicalize(home.join(compat.to_string()).join("packages"))?;
    Ok((home, packages))
}

/// The recorded digests of each project's answers.
fn oracle(root: &Path) -> Result<BTreeMap<String, Vec<(String, String)>>> {
    let oracle = fs::read_to_string(root.join(ORACLE))
        .context("run `cargo xtask oracle corpus-lsp` first")?;
    let mut expected: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for line in oracle
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("case\t"))
    {
        let [case, answer, digest] = line.split('\t').collect::<Vec<_>>()[..] else {
            bail!("malformed row `{line}`");
        };
        expected
            .entry(case.to_owned())
            .or_default()
            .push((answer.to_owned(), digest.to_owned()));
    }
    Ok(expected)
}

/// `cargo xtask oracle corpus-lsp`.
pub fn record(
    root: &Path,
    refs: &References,
    jobs: Option<usize>,
    only: Option<&str>,
) -> Result<()> {
    let compat = Version::parse("0.19.1")?;
    let mut projects = projects(root)?;
    projects.retain(|p| only.is_none_or(|o| p.case.contains(o)));
    let (home, packages) = home(compat)?;
    let scratch = sandbox()?;
    let tools = crate::oracle::lsp::tools(
        refs,
        crate::oracle::lsp::server_path()?,
        &scratch,
        home,
        packages,
    )?;
    let jobs = jobs.unwrap_or(2).max(1);
    let next = AtomicUsize::new(0);
    // Recording some projects keeps the others' rows.
    let mut kept = BTreeMap::new();
    if only.is_some() {
        kept = oracle(root).unwrap_or_default();
        for project in &projects {
            kept.remove(&project.case);
        }
    }
    let rows = Mutex::new(kept);
    let failures = Mutex::new(Vec::new());
    thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                while let Some(project) = projects.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let name = sandbox_name(&project.case);
                    let recorded = (|| -> Result<Value> {
                        let dir = scratch.join("projects").join(&name);
                        copy(project, &dir)?;
                        let case = lsp::Case {
                            name: name.clone(),
                            dir,
                            session: project.session.clone(),
                        };
                        std::panic::catch_unwind(AssertUnwindSafe(|| {
                            crate::oracle::lsp::record(&tools, &scratch.join("work"), &case)
                        }))
                        .unwrap_or_else(|_| bail!("the session broke"))
                    })();
                    match recorded {
                        Ok(transcript) => {
                            let out = scratch.join("elm-language-server");
                            let _ = fs::create_dir_all(&out);
                            let _ = fs::write(
                                out.join(format!("{name}.json")),
                                lsp::to_text(&transcript),
                            );
                            rows.lock()
                                .unwrap()
                                .insert(project.case.clone(), digests(&transcript));
                            eprintln!("  {}", project.case);
                        }
                        Err(e) => failures
                            .lock()
                            .unwrap()
                            .push(format!("{}: {e:#}", project.case)),
                    }
                }
            });
        }
    });
    let mut text = String::from(
        "# @generated by `cargo xtask oracle corpus-lsp`; regenerate, never edit.\ncase\tanswer\tsha256\n",
    );
    for (case, digests) in rows.into_inner().unwrap() {
        for (answer, digest) in digests {
            writeln!(text, "{case}\t{answer}\t{digest}")?;
        }
    }
    fs::create_dir_all(root.join(DIR))?;
    fs::write(root.join(ORACLE), text)?;
    let failures = failures.into_inner().unwrap();
    eprintln!("recorded {} projects", projects.len() - failures.len());
    if !failures.is_empty() {
        bail!(
            "{} projects failed:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
    Ok(())
}

/// The store of the development cache, and what to write for each
/// release's sources in it.
fn store() -> Result<(Store, Vec<(String, String)>)> {
    let dir = cache_dir()?.join("nomi/packages");
    let mut replacements = Vec::new();
    for (name, version, sources) in crate::oracle::corpus_runs::releases(&dir)? {
        let to = format!("<packages>/{name}/{version}");
        let real = fs::canonicalize(&sources).unwrap_or(sources.clone());
        for path in [real, sources] {
            if let Ok(uri) = Url::from_directory_path(&path) {
                replacements.push((uri.as_str().trim_end_matches('/').to_owned(), to.clone()));
            }
            replacements.push((path.display().to_string(), to.clone()));
        }
    }
    Ok((Store::new(dir), replacements))
}

/// `cargo xtask corpus lsp`.
pub fn check(root: &Path, record: bool, only: Option<&str>) -> Result<()> {
    let expected = oracle(root)?;
    let mut projects = projects(root)?;
    projects.retain(|p| expected.contains_key(&p.case) && only.is_none_or(|o| p.case.contains(o)));
    let (store, mut replacements) = store()?;
    // elm-language-server's package cache, written as nomi's store is.
    let (_, packages) = home(Version::parse("0.19.1")?)?;
    for entry in fs::read_dir(&packages)?.flatten() {
        for project in fs::read_dir(entry.path()).into_iter().flatten().flatten() {
            for version in fs::read_dir(project.path()).into_iter().flatten().flatten() {
                let name = format!(
                    "{}/{}",
                    entry.file_name().to_string_lossy(),
                    project.file_name().to_string_lossy()
                );
                let to = format!(
                    "<packages>/{name}/{}",
                    version.file_name().to_string_lossy()
                );
                replacements.push((version.path().display().to_string(), to));
            }
        }
    }
    let scratch = sandbox()?;
    let out = scratch.join("nomi");
    fs::create_dir_all(&out)?;
    // The sessions, for reading a transcript's answers against.
    let sessions = scratch.join("sessions");
    fs::create_dir_all(&sessions)?;
    for project in &projects {
        let steps: Vec<Value> = (project.session.requests.iter())
            .map(|s| json!({ "method": s.method, "file": s.file, "at": format!("{:?}", s.at) }))
            .collect();
        let name = sandbox_name(&project.case);
        fs::write(
            sessions.join(format!("{name}.json")),
            serde_json::to_string_pretty(&steps)?,
        )?;
    }
    let memo = Memo::new();
    let mut outcomes = Vec::new();
    for project in &projects {
        let name = sandbox_name(&project.case);
        let dir = scratch.join("replay").join(&name);
        copy(project, &dir)?;
        let work = nomi_engine::canonical(&dir)?;
        let root_uri = Url::from_directory_path(&work).map_err(|()| anyhow::anyhow!("a path"))?;
        let root_uri = root_uri.as_str().trim_end_matches('/').to_owned();
        let (server, client) = Connection::memory();
        let (s, m) = (store.clone(), memo.clone());
        let handle = thread::spawn(move || nomi_lsp::serve(&server, s, m));
        let timing = Timing {
            quiet: Duration::from_millis(300),
            answer: Duration::from_secs(120),
            publishes: true,
        };
        let session = project.session.clone();
        let transcript = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let driver = Driver::start(client, &work, &root_uri, &session, timing);
            driver.run(&session)
        }));
        let _ = handle.join();
        let mut subs = replacements.clone();
        subs.push((root_uri, "<project>".to_owned()));
        subs.push((work.display().to_string(), "<project>".to_owned()));
        let found = match transcript {
            Ok(t) => {
                let t = lsp::tidy(t, &subs);
                let _ = fs::write(out.join(format!("{name}.json")), lsp::to_text(&t));
                digests(&t)
            }
            Err(_) => Vec::new(),
        };
        for (answer, digest) in &expected[&project.case] {
            let got = found.iter().find(|(a, _)| a == answer).map(|(_, d)| d);
            let result = if got == Some(digest) {
                Ok(())
            } else {
                Err(format!(
                    "differs; compare {name}.json in {} and nomi/",
                    scratch.join("elm-language-server").display()
                ))
            };
            outcomes.push((format!("{}/{answer}", project.case), result));
        }
        eprintln!("  {}", project.case);
    }
    outcomes.sort_by(|a, b| a.0.cmp(&b.0));
    let run: Vec<String> = projects.iter().map(|p| format!("{}/", p.case)).collect();
    Suite::open(root.join(DIR))
        .within(|case| run.iter().any(|p| case.starts_with(p.as_str())))
        .run_by("cargo xtask corpus lsp --record", record)
        .finish(outcomes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_skip_comments_and_strings() {
        let text = "{-| Doc with words\n-}\nmodule Main exposing (main)\n\n-- a comment\nmain =\n    text \"not a name\" ++ x_1 ++ \"\"\"not\n\" either\"\"\"\n";
        let found: Vec<(u32, u32)> = names(text);
        let words: Vec<&str> = found
            .iter()
            .map(|(l, c)| {
                let line = text.lines().nth(*l as usize).unwrap();
                let rest = &line[*c as usize..];
                &rest[..rest
                    .find(|ch: char| !ch.is_alphanumeric() && ch != '_')
                    .unwrap_or(rest.len())]
            })
            .collect();
        assert_eq!(
            words,
            ["module", "Main", "exposing", "main", "main", "text", "x_1"]
        );
    }
}
