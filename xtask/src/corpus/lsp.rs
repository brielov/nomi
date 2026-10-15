//! The corpus differential suite of the language server: a
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
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use lsp_server::Connection;
use lsp_types::Url;
use nomi_base::Compat;
use nomi_engine::Store;
use nomi_engine::index::Memo;
use nomi_testkit::lsp::{self, At, Driver, Method, Position, Session, Step, Timing};
use nomi_testkit::ratchet::Suite;
use nomi_testkit::sha256_hex as sha256;
use serde_json::{Map, Value, json};

use super::{apps, compat, copy_project, corpus_dir, packages, runs};
use crate::oracle::corpus_runs::{self, Cell};
use crate::oracle::lsp::Tools;
use crate::oracle::{cache_dir, generated};
use crate::references::References;
use nomi_testkit::parallel_on;

const DIR: &str = "corpus/lsp";
const ORACLE: &str = "corpus/lsp/oracle.tsv";
/// The seed every choice hashes with; changing it changes every session.
const SEED: &str = "nomi corpus-lsp 1";
/// How many packages join the apps.
const PACKAGES: usize = 100;
const MODULES: usize = 3;
const NAMES: usize = 8;
const COMPLETIONS: usize = 2;
/// The release elm-language-server is built around.
const COMPAT: Compat = Compat::V0_19_1;

/// A project of the suite: its case, its directory, and its `elm.json`
/// as 0.19.1 reads it.
#[derive(Clone)]
pub(crate) struct Project {
    case: String,
    root: PathBuf,
    manifest: String,
}

/// A number from the seed and `parts`, for choosing.
fn pick(parts: &[&str]) -> u64 {
    let mut text = String::from(SEED);
    for part in parts {
        text.push('\u{0}');
        text.push_str(part);
    }
    u64::from_str_radix(&sha256(text.as_bytes())[..16], 16).expect("16 hex digits")
}

/// The suite's generated projects, without their sessions: the apps elm
/// accepts and a sample of the packages.
fn catalog(root: &Path) -> Result<Vec<Project>> {
    let classification = compat::load(root)?;
    let corpus = corpus_dir()?;
    let check = (runs::load(root, corpus_runs::Suite::Check)?.remove(&COMPAT))
        .context("corpus/check/oracle.tsv has no elm 0.19.1")?;
    let accepted = |case: &str| check.get(case) == Some(&Cell::Ok);
    let mut found = Vec::new();
    for app in apps::load(root)?.app {
        let case = format!("app/{}", app.name);
        if !accepted(&case) {
            continue;
        }
        let manifest = compat::sandbox_manifest(&classification, &app, COMPAT)?;
        // A source directory outside the app's root would have to come
        // along outside the session's copy too, which the recorder's copy
        // leaves behind; inside, it's inside `.` as well.
        let elm: Value = serde_json::from_str(&manifest)?;
        let outside = (elm["source-directories"].as_array().into_iter().flatten())
            .any(|d| d.as_str().is_some_and(|d| d.starts_with("..")));
        if outside {
            continue;
        }
        let root = apps::app_dir(&corpus, &app).join(&app.root);
        found.push(Project {
            case,
            root,
            manifest,
        });
    }
    let mut candidates = Vec::new();
    for package in packages::load(root)?.packages {
        let case = format!("{}@{}", package.name, package.version);
        let admitted = classification.admits(&package.name, &package.version, COMPAT);
        if admitted && accepted(&case) {
            candidates.push((pick(&[&case]), case, package));
        }
    }
    candidates.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    for (_, case, package) in candidates.into_iter().take(PACKAGES) {
        let root = packages::package_dir(&corpus, &package);
        let manifest = fs::read_to_string(root.join("elm.json"))?;
        found.push(Project {
            case,
            root,
            manifest,
        });
    }
    Ok(found)
}

/// The suite's projects whose case contains `only`, each with its
/// session. An editor's trace of an app is one more project, the app's
/// again, as `trace/<app>`.
fn sessions(root: &Path, only: Option<&str>) -> Result<Vec<(Project, Session)>> {
    let trace = |app: &str| root.join(super::trace::DIR).join(format!("{app}.json"));
    let projects = catalog(root)?;
    let traces: Vec<Project> = (projects.iter())
        .filter_map(|project| {
            let app = project.case.strip_prefix("app/")?;
            trace(app).is_file().then(|| Project {
                case: format!("trace/{app}"),
                ..project.clone()
            })
        })
        .collect();
    (projects.into_iter().chain(traces))
        .filter(|project| only.is_none_or(|o| project.case.contains(o)))
        .map(|project| {
            let session = match project.case.strip_prefix("trace/") {
                Some(app) => {
                    let path = trace(app);
                    let text = fs::read_to_string(&path)?;
                    serde_json::from_str(&text).with_context(|| path.display().to_string())?
                }
                None => session(&project)?,
            };
            Ok((project, session))
        })
        .collect()
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

/// A request of the generated sessions: `method` on `file`, at `at`.
fn step(method: &str, file: &str, at: Option<(u32, u32)>) -> Step {
    let method = Method::from(method.to_owned());
    let mut params = Map::new();
    // As VS Code asks.
    if method == Method::References {
        params.insert("context".to_owned(), json!({ "includeDeclaration": true }));
    }
    Step {
        method,
        file: Some(file.to_owned()),
        at: at.map(|(line, character)| At::Exact(Position { line, character })),
        params,
        resolve: false,
        create: BTreeMap::new(),
    }
}

/// The session generated for a project.
fn session(project: &Project) -> Result<Session> {
    let Project {
        case,
        root,
        manifest,
    } = project;
    let mut modules: Vec<(u64, String)> = (modules(root, manifest).into_iter())
        .map(|m| (pick(&[case, &m]), m))
        .collect();
    modules.sort();
    let opened: Vec<String> = modules.into_iter().take(MODULES).map(|(_, m)| m).collect();
    let mut requests = Vec::new();
    for module in &opened {
        let text = fs::read_to_string(root.join(module))?;
        requests.push(step("textDocument/documentSymbol", module, None));
        requests.push(step("textDocument/foldingRange", module, None));
        let mut at: Vec<(u64, (u32, u32))> = (names(&text).into_iter())
            .map(|p| (pick(&[case, module, &format!("{}:{}", p.0, p.1)]), p))
            .collect();
        at.sort_unstable();
        for (_, p) in at.iter().take(NAMES) {
            for method in [
                "textDocument/hover",
                "textDocument/definition",
                "textDocument/references",
            ] {
                requests.push(step(method, module, Some(*p)));
            }
        }
        for (_, (line, character)) in at.iter().skip(NAMES).take(COMPLETIONS) {
            // Just after the name's first character, as one types it.
            let at = Some((*line, character + 1));
            requests.push(step("textDocument/completion", module, at));
        }
    }
    Ok(Session {
        open: opened,
        requests,
        ..Session::default()
    })
}

/// A project copied for a session: its root, as [`copy_project`] copies
/// it, with its `elm.json` as 0.19.1 reads it.
pub(crate) fn copy(project: &Project, to: &Path) -> Result<()> {
    copy_project(&project.root, to)?;
    fs::write(to.join("elm.json"), &project.manifest)?;
    Ok(())
}

/// Each answer's digest, the diagnostics first, as the oracle stores them:
/// the answer, as `<index> <method>`, and its digest.
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

pub(crate) fn sandbox() -> Result<PathBuf> {
    Ok(cache_dir()?.join("sandboxes/corpus/corpus-lsp"))
}

/// elm-language-server as the suite runs it: with the `ELM_HOME` the
/// corpus suites share, and a `PATH` of its own under `scratch`.
pub(crate) fn server(refs: &References, scratch: &Path) -> Result<Tools> {
    let (home, packages) = home()?;
    let server = crate::oracle::lsp::server_path()?;
    crate::oracle::lsp::tools(refs, server, scratch, home, packages)
}

/// The project of `case`, without its session.
pub(crate) fn project(root: &Path, case: &str) -> Result<Project> {
    (catalog(root)?.into_iter())
        .find(|p| p.case == case)
        .with_context(|| format!("{case} isn't one of the suite's projects"))
}

/// The `ELM_HOME` the corpus suites share, as 0.19.1's, holding every
/// release the development cache has: the home, and its package cache.
fn home() -> Result<(PathBuf, PathBuf)> {
    let home = cache_dir()?
        .join("sandboxes/corpus/home")
        .join(COMPAT.as_str());
    let releases = corpus_runs::releases(&cache_dir()?.join("nomi/packages"))?;
    ensure!(
        !releases.is_empty(),
        "the development cache has no releases; run `cargo xtask corpus names`"
    );
    let packages = fs::canonicalize(crate::oracle::sandbox::elm_home(&home, COMPAT, &releases)?)?;
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
pub(crate) fn record(
    root: &Path,
    refs: &References,
    jobs: Option<usize>,
    only: Option<&str>,
) -> Result<()> {
    let projects = sessions(root, only)?;
    let scratch = sandbox()?;
    let tools = server(refs, &scratch)?;
    let recorded = parallel_on(
        jobs.unwrap_or(2),
        &projects,
        |(project, session)| -> Result<_> {
            let name = sandbox_name(&project.case);
            let dir = scratch.join("projects").join(&name);
            copy(project, &dir)?;
            let case = lsp::Case {
                name: name.clone(),
                dir,
                session: session.clone(),
            };
            let transcript = std::panic::catch_unwind(AssertUnwindSafe(|| {
                crate::oracle::lsp::record(&tools, &scratch.join("work"), &case)
            }))
            .unwrap_or_else(|_| bail!("the session broke"))?;
            let out = scratch.join("elm-language-server");
            let _ = fs::create_dir_all(&out);
            let _ = fs::write(out.join(format!("{name}.json")), lsp::to_text(&transcript));
            eprintln!("  {}", project.case);
            Ok(digests(&transcript))
        },
    );
    // Recording some projects keeps the others' rows.
    let mut rows = if only.is_some() {
        oracle(root).unwrap_or_default()
    } else {
        BTreeMap::new()
    };
    let mut failures = Vec::new();
    for ((project, _), recorded) in projects.iter().zip(recorded) {
        rows.remove(&project.case);
        match recorded {
            Ok(digests) => {
                rows.insert(project.case.clone(), digests);
            }
            Err(e) => failures.push(format!("{}: {e:#}", project.case)),
        }
    }
    let mut text = format!("{}\ncase\tanswer\tsha256\n", generated("corpus-lsp"));
    for (case, digests) in rows {
        for (answer, digest) in digests {
            writeln!(text, "{case}\t{answer}\t{digest}")?;
        }
    }
    fs::create_dir_all(root.join(DIR))?;
    fs::write(root.join(ORACLE), text)?;
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

/// What to write for each release's files, as the suite's transcripts
/// give them: wherever the development cache's store keeps them, as a
/// path or a URI, then wherever elm-language-server's package cache
/// `packages` does.
fn replacements(packages: &Path) -> Result<Vec<(String, String)>> {
    let releases = corpus_runs::releases(&cache_dir()?.join("nomi/packages"))?;
    let to = |release: &crate::oracle::sandbox::Release| {
        format!("<packages>/{}/{}", release.name, release.version)
    };
    let mut replacements = Vec::new();
    for release in &releases {
        let real = fs::canonicalize(&release.dir).unwrap_or_else(|_| release.dir.clone());
        for path in [&real, &release.dir] {
            if let Ok(uri) = Url::from_directory_path(path) {
                replacements.push((uri.as_str().trim_end_matches('/').to_owned(), to(release)));
            }
            replacements.push((path.display().to_string(), to(release)));
        }
    }
    for release in &releases {
        let cached = packages
            .join(&release.name)
            .join(release.version.to_string());
        replacements.push((cached.display().to_string(), to(release)));
    }
    Ok(replacements)
}

/// `cargo xtask corpus lsp`.
pub(crate) fn check(root: &Path, record: bool, only: Option<&str>) -> Result<()> {
    let expected = oracle(root)?;
    let mut projects = sessions(root, only)?;
    for (project, _) in projects
        .iter()
        .filter(|(p, _)| !expected.contains_key(&p.case))
    {
        eprintln!("not recorded, so not checked: {}", project.case);
    }
    projects.retain(|(p, _)| expected.contains_key(&p.case));
    let (_, packages) = home()?;
    let replacements = replacements(&packages)?;
    let store = Store::new(cache_dir()?.join("nomi/packages"));
    let scratch = sandbox()?;
    let out = scratch.join("nomi");
    fs::create_dir_all(&out)?;
    // The sessions, for reading a transcript's answers against.
    let sessions = scratch.join("sessions");
    fs::create_dir_all(&sessions)?;
    for (project, session) in &projects {
        let steps: Vec<Value> = (session.requests.iter())
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
    for (project, session) in &projects {
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
        let transcript = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let driver = Driver::start(client, &work, &root_uri, session, timing);
            driver.run(session)
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
        let recorded = &expected[&project.case];
        let unrecorded: Vec<&String> = (found.iter().map(|(a, _)| a))
            .filter(|a| !recorded.iter().any(|(r, _)| r == *a))
            .collect();
        if !unrecorded.is_empty() {
            bail!(
                "{}: nomi answered {unrecorded:?}, which elm-language-server's recording doesn't have",
                project.case
            );
        }
        for (answer, digest) in recorded {
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
    let run: Vec<String> = projects
        .iter()
        .map(|(p, _)| format!("{}/", p.case))
        .collect();
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
