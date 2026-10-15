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
use std::iter::Peekable;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::str::Chars;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use lsp_server::Connection;
use nomi_base::Compat;
use nomi_engine::Store;
use nomi_engine::index::Memo;
use nomi_project::Outline;
use nomi_testkit::lsp::{self, At, Driver, Method, Position, Session, Step, Timing};
use nomi_testkit::ratchet::Suite;
use nomi_testkit::sha256_hex as sha256;
use serde_json::{Map, Value, json};

use super::{apps, compat, copy_project, corpus_dir, packages, runs};
use crate::oracle::corpus_runs::{self, Cell, cache_releases, corpus_home};
use crate::oracle::lsp::Tools;
use crate::oracle::sandbox::Release;
use crate::oracle::{cache_dir, generated};
use crate::references::References;
use nomi_testkit::{files_below, parallel_on};

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

/// A project of the suite: its case, its directory, its `elm.json` as
/// 0.19.1 reads it, and the source directories that names.
#[derive(Clone)]
pub(crate) struct Project {
    case: String,
    root: PathBuf,
    manifest: String,
    sources: Vec<String>,
}

/// The source directories `manifest` names, as 0.19.1 reads it.
fn source_dirs(manifest: &str) -> Result<Vec<String>> {
    let outline = Outline::read(manifest.as_bytes(), COMPAT).map_err(|e| anyhow!("{e:?}"))?;
    Ok(outline.source_dirs())
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
        let case = app.case();
        if !accepted(&case) {
            continue;
        }
        let manifest = compat::sandbox_manifest(&classification, &app, COMPAT)?;
        // A source directory outside the app's root would have to come
        // along outside the session's copy too, which the recorder's copy
        // leaves behind; inside, it's inside `.` as well.
        let sources = source_dirs(&manifest).with_context(|| case.clone())?;
        if sources.iter().any(|d| d.starts_with("..")) {
            continue;
        }
        let root = apps::app_dir(&corpus, &app).join(&app.root);
        found.push(Project {
            case,
            root,
            manifest,
            sources,
        });
    }
    let mut candidates = Vec::new();
    for package in packages::load(root)?.packages {
        let case = package.case();
        let admitted = classification.admits(&package.name, &package.version, COMPAT);
        if admitted && accepted(&case) {
            candidates.push((pick(&[&case]), case, package));
        }
    }
    candidates.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    for (_, case, package) in candidates.into_iter().take(PACKAGES) {
        let root = packages::package_dir(&corpus, &package);
        let manifest = fs::read_to_string(root.join("elm.json"))?;
        let sources = source_dirs(&manifest).with_context(|| case.clone())?;
        found.push(Project {
            case,
            root,
            manifest,
            sources,
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

/// The modules of `project`'s source directories, by path from its root.
fn modules(project: &Project) -> Vec<String> {
    let root = &project.root;
    let mut found = Vec::new();
    for dir in &project.sources {
        let dir = root.join(dir);
        for file in files_below(&dir).unwrap_or_default() {
            let path = dir.join(file);
            if path.extension().is_some_and(|e| e == "elm")
                && let Ok(relative) = path.strip_prefix(root)
                && !relative.starts_with("..")
            {
                found.push(relative.to_string_lossy().replace('\\', "/"));
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
    let mut scanner = Scanner {
        chars: text.chars().peekable(),
        line: 0,
        column: 0,
    };
    let mut previous = ' ';
    loop {
        let start = (scanner.line, scanner.column);
        let Some(c) = scanner.next() else {
            return found;
        };
        match c {
            '-' if scanner.chars.peek() == Some(&'-') => scanner.skip_line_comment(),
            '{' if scanner.chars.peek() == Some(&'-') => scanner.skip_block_comment(),
            '"' if scanner.starts_with("\"\"") => scanner.skip_multiline_string(),
            '"' | '\'' => scanner.skip_string(c),
            c if c.is_alphabetic() && !previous.is_alphanumeric() && previous != '_' => {
                found.push(start);
                scanner.skip_name();
                previous = 'a';
                continue;
            }
            _ => {}
        }
        previous = c;
    }
}

/// A walk over a module's text, knowing the LSP position of what's next.
struct Scanner<'a> {
    chars: Peekable<Chars<'a>>,
    line: u32,
    /// The UTF-16 column.
    column: u32,
}

impl Scanner<'_> {
    /// The next character, the position moved past it.
    fn next(&mut self) -> Option<char> {
        let c = self.chars.next()?;
        if c == '\n' {
            self.line += 1;
            self.column = 0;
        } else {
            self.column += c.len_utf16() as u32;
        }
        Some(c)
    }

    /// Whether what's next starts with `prefix`.
    fn starts_with(&self, prefix: &str) -> bool {
        (self.chars.clone())
            .take(prefix.chars().count())
            .eq(prefix.chars())
    }

    /// The rest of a line comment, to the line's end.
    fn skip_line_comment(&mut self) {
        while self.chars.peek().is_some_and(|&c| c != '\n') {
            self.next();
        }
    }

    /// The rest of a block comment whose `{` is behind, with the comments
    /// it nests. Its `-` opens it, and nests nothing.
    fn skip_block_comment(&mut self) {
        self.next();
        let mut depth = 1;
        let mut last = ' ';
        while let Some(c) = self.next() {
            match (last, c) {
                ('{', '-') => depth += 1,
                ('-', '}') => {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
            last = c;
        }
    }

    /// The rest of a multiline string whose first `"` is behind, to its
    /// closing `\"\"\"`.
    fn skip_multiline_string(&mut self) {
        self.next();
        self.next();
        let mut quotes = 0;
        while let Some(c) = self.next() {
            quotes = if c == '"' { quotes + 1 } else { 0 };
            if quotes == 3 {
                return;
            }
        }
    }

    /// The rest of a string or character literal opened by `quote`.
    fn skip_string(&mut self, quote: char) {
        let mut escaped = false;
        while let Some(c) = self.next() {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == quote {
                return;
            }
        }
    }

    /// The rest of a name whose first letter is behind.
    fn skip_name(&mut self) {
        while self
            .chars
            .peek()
            .is_some_and(|&c| c.is_alphanumeric() || c == '_')
        {
            self.next();
        }
    }
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
    let Project { case, root, .. } = project;
    let mut modules: Vec<(u64, String)> = (modules(project).into_iter())
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
    let (home, packages) = home(&cache_releases()?)?;
    let server = crate::oracle::lsp::server_path()?;
    crate::oracle::lsp::tools(refs, server, scratch, home, packages)
}

/// The project of `case`, without its session.
pub(crate) fn project(root: &Path, case: &str) -> Result<Project> {
    (catalog(root)?.into_iter())
        .find(|p| p.case == case)
        .with_context(|| format!("{case} isn't one of the suite's projects"))
}

/// The `ELM_HOME` the corpus suites share, as 0.19.1's, holding
/// `releases`: the home, and its package cache.
fn home(releases: &[Release]) -> Result<(PathBuf, PathBuf)> {
    let (home, packages) = corpus_home(COMPAT, releases)?;
    Ok((home, fs::canonicalize(packages)?))
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
fn replacements(packages: &Path, releases: &[Release]) -> Vec<(String, String)> {
    let to = |release: &Release| format!("<packages>/{}/{}", release.name, release.version);
    let mut replacements = Vec::new();
    for release in releases {
        let real = fs::canonicalize(&release.dir).unwrap_or_else(|_| release.dir.clone());
        replacements.extend(lsp::dir_replacements(&[&real, &release.dir], &to(release)));
    }
    for release in releases {
        let cached = packages
            .join(&release.name)
            .join(release.version.to_string());
        replacements.push((cached.display().to_string(), to(release)));
    }
    replacements
}

/// Writes each session's requests in `dir`, for reading a transcript's
/// answers against.
fn write_sessions(dir: &Path, projects: &[(Project, Session)]) -> Result<()> {
    fs::create_dir_all(dir)?;
    for (project, session) in projects {
        let name = sandbox_name(&project.case);
        let steps = serde_json::to_string_pretty(&session.requests)?;
        fs::write(dir.join(format!("{name}.json")), steps)?;
    }
    Ok(())
}

/// nomi's answers to `session` on a copy of `project` in `scratch`, by
/// their digests, with the transcript written to `scratch`'s `nomi/`;
/// none when the driver panicked.
fn replay(
    project: &Project,
    session: &Session,
    store: &Store,
    memo: &Memo,
    replacements: &[(String, String)],
    scratch: &Path,
) -> Result<Vec<(String, String)>> {
    let name = sandbox_name(&project.case);
    let dir = scratch.join("replay").join(&name);
    copy(project, &dir)?;
    let work = nomi_engine::canonical(&dir)?;
    let (server, client) = Connection::memory();
    let (s, m) = (store.clone(), memo.clone());
    let handle = thread::spawn(move || nomi_lsp::serve(&server, s, m));
    let timing = Timing {
        quiet: Duration::from_millis(300),
        answer: Duration::from_secs(120),
        publishes: true,
    };
    let transcript = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let driver = Driver::start(client, &work, session, timing);
        driver.run(session)
    }));
    let _ = handle.join();
    let Ok(transcript) = transcript else {
        return Ok(Vec::new());
    };
    let mut subs = replacements.to_vec();
    subs.extend(lsp::dir_replacements(&[&work], "<project>"));
    let transcript = lsp::tidy(transcript, &subs);
    let _ = fs::write(
        scratch.join("nomi").join(format!("{name}.json")),
        lsp::to_text(&transcript),
    );
    Ok(digests(&transcript))
}

/// Each answer `case`'s recording has, passing when nomi's `found` digest
/// is the recorded one, and failing with `differs` otherwise. An answer
/// the recording lacks fails the run: the session asked something
/// elm-language-server never answered, so the recording is stale.
fn compare(
    case: &str,
    recorded: &[(String, String)],
    found: &[(String, String)],
    differs: &str,
) -> Result<Vec<(String, Result<(), String>)>> {
    let unrecorded: Vec<&String> = (found.iter().map(|(a, _)| a))
        .filter(|a| !recorded.iter().any(|(r, _)| r == *a))
        .collect();
    if !unrecorded.is_empty() {
        bail!(
            "{case}: nomi answered {unrecorded:?}, which elm-language-server's recording doesn't have"
        );
    }
    Ok((recorded.iter())
        .map(|(answer, digest)| {
            let got = found.iter().find(|(a, _)| a == answer).map(|(_, d)| d);
            let result = if got == Some(digest) {
                Ok(())
            } else {
                Err(differs.to_owned())
            };
            (format!("{case}/{answer}"), result)
        })
        .collect())
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
    let releases = cache_releases()?;
    let (_, packages) = home(&releases)?;
    let replacements = replacements(&packages, &releases);
    let store = Store::new(cache_dir()?.join("nomi/packages"));
    let scratch = sandbox()?;
    fs::create_dir_all(scratch.join("nomi"))?;
    write_sessions(&scratch.join("sessions"), &projects)?;
    let memo = Memo::new();
    let mut outcomes = Vec::new();
    for (project, session) in &projects {
        let found = replay(project, session, &store, &memo, &replacements, &scratch)?;
        let differs = format!(
            "differs; compare {}.json in {} and nomi/",
            sandbox_name(&project.case),
            scratch.join("elm-language-server").display()
        );
        let recorded = &expected[&project.case];
        outcomes.extend(compare(&project.case, recorded, &found, &differs)?);
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
    fn an_answer_the_recording_lacks_fails_the_run() {
        let pair = |a: &str, d: &str| (a.to_owned(), d.to_owned());
        let recorded = [pair("00 hover", "x"), pair("01 definition", "y")];
        let found = [pair("00 hover", "x"), pair("01 definition", "z")];
        let outcomes = compare("app/a", &recorded, &found, "differs").unwrap();
        assert_eq!(
            outcomes,
            [
                ("app/a/00 hover".to_owned(), Ok(())),
                ("app/a/01 definition".to_owned(), Err("differs".to_owned())),
            ]
        );
        let missing = compare("app/a", &recorded, &found[..1], "differs").unwrap();
        assert_eq!(missing[1].1, Err("differs".to_owned()));
        let extra = [pair("00 hover", "x"), pair("02 references", "w")];
        assert!(compare("app/a", &recorded, &extra, "differs").is_err());
    }

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
