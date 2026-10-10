//! Whether nomi makes of every corpus project what each elm made of it,
//! as `cargo xtask oracle corpus-docs`, `corpus-check`, `corpus-graphs`
//! and `corpus-objects` recorded:
//! `cargo xtask corpus docs` wants the same `docs.json` of every package,
//! byte for byte (plan 7.4a), and `cargo xtask corpus check` the silence
//! of `nomi check` on every app and package elm accepted (plan 7.5);
//! `cargo xtask corpus graphs` the same optimized graph of every module
//! elm built (plan 8.4a); `cargo xtask corpus objects` the same graph of
//! the dependencies as `o.dat` holds (plan 8.4b); `cargo xtask corpus
//! javascript` the same JavaScript of every app in each mode (plan
//! 8.11); and all the same
//! `--report=json` wherever elm
//! rejected the project. Each
//! project's dependencies are solved and downloaded as `corpus names`
//! does, so this runs by hand, never in CI. A project elm couldn't build
//! for want of a release the cache lacked is skipped.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use nomi_base::Compat;
use nomi_compiler::interface::Interface;
use nomi_compiler::optimized::binary;
use nomi_engine::check::{self, Checked, Style};
use nomi_engine::console::{Console, Personality};
use nomi_engine::graph::Foreigns;
use nomi_engine::{Vfs, javascript, names};
use nomi_project::{ModuleName, Name, Outline};

use super::names::{Project, solve};
use super::{compat, parallel};
use crate::download::sha256;
use crate::oracle::corpus_runs::{self, EXCLUDED, OK, Suite};

pub fn check(root: &Path, suite: Suite, only: Option<&str>) -> Result<()> {
    let oracle = load(root, suite)?;
    let classification = compat::load(root)?;
    let mut matching = 0;
    let mut skipped: Vec<String> = Vec::new();
    let mut failures: Vec<(String, String)> = Vec::new();
    for v in &classification.compats {
        let c = (Compat::ALL.into_iter())
            .find(|c| c.to_string() == *v)
            .with_context(|| format!("nomi doesn't emulate elm {v}"))?;
        let (apps, packages) = (suite != Suite::Docs, suite != Suite::JavaScript);
        let solving = solve(root, &classification, c, v, only, apps, packages)?;
        // Dependencies without a solution stop elm before any module too.
        for (case, why) in solving.failures {
            let elm = oracle.get(&(package_case(&case), format!("elm {v}")));
            if elm.is_some_and(|cell| cell.starts_with("error ")) {
                matching += 1;
            } else {
                failures.push((case, why));
            }
        }
        skipped.extend(solving.unavailable);
        let mut memo = HashMap::new();
        for chunk in solving.solved.chunks(64) {
            let mut ready = Vec::new();
            for (project, sources) in chunk {
                let key = package_case(&project.case);
                let Some(expected) = oracle.get(&(key.clone(), format!("elm {v}"))) else {
                    failures.push((project.case.clone(), "not in the oracle".to_owned()));
                    continue;
                };
                if expected == EXCLUDED || expected.starts_with("error ") || expected == "crash" {
                    skipped.push(format!("{}: elm said {expected}", project.case));
                    continue;
                }
                match names::package_interfaces_with(c, sources, &mut memo) {
                    Ok(interfaces) => ready.push((project, sources, interfaces, expected)),
                    Err(why) => {
                        let why: String = why.chars().take(300).collect();
                        failures.push((project.case.clone(), why));
                    }
                }
            }
            let results = parallel(&ready, |(project, sources, interfaces, _)| {
                run(suite, c, project, sources, interfaces)
            });
            for ((project, _, _, expected), result) in ready.iter().zip(results) {
                match result {
                    Ok(cell) if &cell == *expected => matching += 1,
                    Ok(cell) => failures.push((
                        project.case.clone(),
                        format!("nomi: {cell}\n  elm:  {expected}"),
                    )),
                    Err(why) => failures.push((project.case.clone(), why)),
                }
            }
            eprintln!("  {c}: {matching} matching, {} failing", failures.len());
        }
    }
    for why in &skipped {
        eprintln!("skipped: {why}");
    }
    // Runs where elm's own output is wrong, each checked by hand, by case.
    let expected_path = root.join(suite.file()).with_file_name("expected.tsv");
    let expected: BTreeMap<String, String> = std::fs::read_to_string(&expected_path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| l.split_once('\t'))
        .map(|(case, why)| (case.to_owned(), why.to_owned()))
        .collect();
    let (known, failures): (Vec<_>, Vec<_>) = failures
        .into_iter()
        .partition(|(case, _)| expected.contains_key(case));
    for (case, why) in &failures {
        eprintln!("FAIL {case}: {why}");
    }
    if only.is_none() {
        for case in expected
            .keys()
            .filter(|c| !known.iter().any(|(k, _)| k == *c))
        {
            eprintln!(
                "now passes, remove it from {}: {case}",
                expected_path.display()
            );
        }
    }
    let (name, kind) = match suite {
        Suite::Docs => ("docs", "packages"),
        Suite::Check => ("check", "projects"),
        Suite::Graphs => ("graphs", "projects"),
        Suite::Objects => ("objects", "projects"),
        Suite::JavaScript => ("javascript", "apps"),
    };
    println!(
        "{name}: {matching} {kind} made as elm makes them, {} where elm is wrong, {} not, {} skipped",
        known.len(),
        failures.len(),
        skipped.len()
    );
    if !failures.is_empty() {
        bail!("{} {kind} differ", failures.len());
    }
    Ok(())
}

/// Writes nomi's `o.dat` for `case` beside the one the oracle kept, where
/// they differ, for `graph-diff` to look into.
fn keep_objects(compat: Compat, case: &str, bytes: &[u8]) {
    let id = package_case(case).replace('/', "__");
    let Ok(cache) = crate::oracle::cache_dir() else {
        return;
    };
    let dir = cache
        .join("sandboxes/corpus/corpus-objects")
        .join(compat.to_string());
    let elm = dir.join("out").join(format!("{id}.o.dat"));
    if std::fs::read(&elm).is_ok_and(|elm| elm != bytes) {
        let _ = std::fs::create_dir_all(dir.join("nomi"));
        let _ = std::fs::write(dir.join("nomi").join(format!("{id}.o.dat")), bytes);
    }
}

/// Writes nomi's JavaScript for `case` in `mode` beside what the oracle
/// kept, where they differ.
fn keep_javascript(compat: Compat, case: &str, mode: &str, js: &[u8]) {
    let id = package_case(case).replace('/', "__");
    let Ok(cache) = crate::oracle::cache_dir() else {
        return;
    };
    let dir = cache
        .join("sandboxes/corpus/corpus-javascript")
        .join(compat.to_string());
    let file = format!("{id}.{mode}.js");
    if std::fs::read(dir.join("out").join(&file)).is_ok_and(|elm| elm != js) {
        let _ = std::fs::create_dir_all(dir.join("nomi"));
        let _ = std::fs::write(dir.join("nomi").join(&file), js);
    }
}

/// Which of `modules` differ from the `.elmo` files the oracle kept for
/// `case`, if it kept them; nomi's are written beside them for a look.
fn differing(compat: Compat, case: &str, modules: &[(String, Vec<u8>)]) -> Option<String> {
    let id = package_case(case).replace('/', "__");
    let sandbox = crate::oracle::cache_dir()
        .ok()?
        .join("sandboxes/corpus/corpus-graphs");
    let elm = sandbox.join(compat.to_string()).join("out").join(&id);
    if !elm.is_dir() {
        return None;
    }
    let nomi = sandbox.join(compat.to_string()).join("nomi").join(&id);
    let _ = std::fs::create_dir_all(&nomi);
    let mut differ = Vec::new();
    for (name, bytes) in modules {
        let file = format!("{}.elmo", name.replace('.', "-"));
        let _ = std::fs::write(nomi.join(&file), bytes);
        if std::fs::read(elm.join(&file)).ok().as_deref() != Some(bytes.as_slice()) {
            differ.push(name.clone());
        }
    }
    Some(differ.join(", "))
}

thread_local! {
    /// Each package's graph as each release makes it, made once a thread:
    /// projects share them.
    static OBJECTS: std::cell::RefCell<HashMap<Compat, nomi_engine::objects::Memo>> =
        Default::default();
}

/// The oracle's cells, by project case and elm.
fn load(root: &Path, suite: Suite) -> Result<BTreeMap<(String, String), String>> {
    let path = root.join(suite.file());
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}; run its oracle", path.display()))?;
    let mut lines = text.lines().filter(|l| !l.starts_with('#'));
    let header: Vec<&str> = lines
        .next()
        .context("an empty oracle")?
        .split('\t')
        .collect();
    let mut cells = BTreeMap::new();
    for line in lines {
        let row: Vec<&str> = line.split('\t').collect();
        for (elm, cell) in header.iter().zip(&row).skip(1) {
            cells.insert((row[0].to_owned(), (*elm).to_owned()), (*cell).to_owned());
        }
    }
    Ok(cells)
}

/// The oracle's case, `author/project@version` or `app/name`, of a case
/// `… on 0.19.x`.
fn package_case(case: &str) -> String {
    case.split(" on ").next().unwrap_or(case).to_owned()
}

/// What `suite`'s `elm make` makes of `project`, as an oracle cell.
fn run(
    suite: Suite,
    compat: Compat,
    project: &Project,
    sources: &[(Name, PathBuf)],
    interfaces: &names::PackageInterfaces,
) -> Result<String, String> {
    let direct: Vec<Name> = match &project.outline {
        Outline::App(app) => app.direct.keys().cloned().collect(),
        Outline::Pkg(pkg) => pkg.deps.keys().cloned().collect(),
    };
    let dirs: BTreeMap<&Name, &PathBuf> = sources.iter().map(|(n, d)| (n, d)).collect();
    let mut exposing = Vec::new();
    for name in &direct {
        let dir = dirs
            .get(name)
            .ok_or_else(|| format!("{name} wasn't downloaded"))?;
        let bytes = std::fs::read(dir.join("elm.json")).map_err(|e| e.to_string())?;
        let Ok(Outline::Pkg(dep)) = Outline::decode(&bytes, compat) else {
            return Err(format!("{name}'s elm.json"));
        };
        exposing.push((
            name.clone(),
            dep.exposed.modules().cloned().collect::<Vec<ModuleName>>(),
        ));
    }
    let visible: HashMap<String, Interface> = names::visible(interfaces, &direct);
    let root = std::fs::canonicalize(&project.root).map_err(|e| e.to_string())?;
    let env = check::env_of(&root, &project.outline, compat, Foreigns::new(exposing))
        .map_err(|r| r.title.to_string())?
        .with_cwd(&root);
    let snapshot = Vfs::new().snapshot();
    let checked = match suite {
        Suite::Docs => {
            let (checked, docs, _) = check::docs(
                &snapshot,
                &env,
                &project.outline,
                Personality::Elm,
                &visible,
            );
            if let Some(docs) = docs {
                return Ok(format!("docs {}", sha256(docs.as_bytes())));
            }
            checked
        }
        Suite::Check => {
            let checked = check::check(
                &snapshot,
                &env,
                &project.outline,
                &project.paths,
                Personality::Elm,
                &visible,
            );
            if matches!(checked, Checked::Fine) {
                return Ok(OK.to_owned());
            }
            checked
        }
        Suite::Objects => {
            let checked = check::check(
                &snapshot,
                &env,
                &project.outline,
                &project.paths,
                Personality::Elm,
                &visible,
            );
            if matches!(checked, Checked::Fine) {
                let graph = OBJECTS.with(|memos| {
                    let mut memos = memos.borrow_mut();
                    let memo = memos.entry(compat).or_default();
                    nomi_engine::objects::build(compat, sources, memo, None).map(|d| d.graph())
                })?;
                let bytes = binary::encode_global_graph(&graph, compat);
                keep_objects(compat, &project.case, &bytes);
                return Ok(format!("objects {}", sha256(&bytes)));
            }
            checked
        }
        Suite::JavaScript => {
            let (checked, optimized) = check::optimize(
                &snapshot,
                &env,
                &project.outline,
                &project.paths,
                Personality::Elm,
                &visible,
                None,
            );
            if matches!(checked, Checked::Fine) {
                let built = OBJECTS.with(|memos| {
                    let mut memos = memos.borrow_mut();
                    let memo = memos.entry(compat).or_default();
                    nomi_engine::objects::build(compat, sources, memo, None)
                })?;
                let dependencies = (built.graph(), built.types());
                let refused = !javascript::debug_uses(compat, &optimized).is_empty();
                let mut hashes = Vec::new();
                for (mode, kind) in [
                    ("dev", javascript::Kind::Dev),
                    ("debug", javascript::Kind::Debug),
                    ("optimize", javascript::Kind::Optimize),
                ] {
                    if refused && kind == javascript::Kind::Optimize {
                        hashes.push("-".to_owned());
                        continue;
                    }
                    let js = javascript::program(
                        compat,
                        kind,
                        "author/project",
                        dependencies.clone(),
                        optimized.clone(),
                    );
                    keep_javascript(compat, &project.case, mode, &js);
                    hashes.push(sha256(&js));
                }
                return Ok(format!("javascript {}", hashes.join(" ")));
            }
            checked
        }
        Suite::Graphs => {
            let (checked, optimized) = check::optimize(
                &snapshot,
                &env,
                &project.outline,
                &project.paths,
                Personality::Elm,
                &visible,
                None,
            );
            if matches!(checked, Checked::Fine) {
                let modules: Vec<(String, Vec<u8>)> = (optimized.graphs.into_iter())
                    .map(|(name, graph)| {
                        (name.to_string(), binary::encode_local_graph(&graph, compat))
                    })
                    .collect();
                let cell = corpus_runs::graphs_cell(modules.clone());
                return Ok(match differing(compat, &project.case, &modules) {
                    Some(why) if !why.is_empty() => format!("{cell} ({why})"),
                    _ => cell,
                });
            }
            checked
        }
    };
    let mut console = Captured::default();
    check::show(&mut console, &root, &checked, compat, Style::Json);
    let root = root.to_str().ok_or("a path that isn't UTF-8")?;
    let report = console.err.trim_end().replace(root, "$ROOT");
    Ok(corpus_runs::cell(&report))
}

#[derive(Default)]
struct Captured {
    err: String,
}

impl Console for Captured {
    fn out(&mut self, _: &str) {}

    fn err(&mut self, text: &str) {
        self.err.push_str(text);
    }

    fn read_line(&mut self) -> io::Result<Option<String>> {
        Ok(None)
    }

    fn read_all(&mut self) -> io::Result<Vec<u8>> {
        Ok(Vec::new())
    }
}

/// `cargo xtask corpus graph-diff`: where the graph in `nomi` first
/// differs from the one in `elm`, both `.elmo` files of release `v`.
pub fn graph_diff(v: &str, nomi: &Path, elm: &Path) -> Result<()> {
    let compat = (Compat::ALL.into_iter())
        .find(|c| c.to_string() == v)
        .with_context(|| format!("nomi doesn't emulate elm {v}"))?;
    // An `o.dat` is a whole graph, read here as a module's without main.
    let read = |path: &Path| -> Result<nomi_compiler::optimized::LocalGraph> {
        let bytes = std::fs::read(path)?;
        if path.extension().is_some_and(|e| e == "dat") {
            let graph = binary::decode_global_graph(&bytes, compat)
                .with_context(|| format!("{} doesn't decode", path.display()))?;
            return Ok(nomi_compiler::optimized::LocalGraph {
                main: None,
                nodes: graph.nodes,
                fields: graph.fields,
            });
        }
        binary::decode_local_graph(&bytes, compat)
            .with_context(|| format!("{} doesn't decode", path.display()))
    };
    let (ours, theirs) = (read(nomi)?, read(elm)?);
    if ours.main != theirs.main {
        println!("main:\n  nomi: {:?}\n  elm:  {:?}", ours.main, theirs.main);
    }
    for (global, node) in &theirs.nodes {
        match ours.nodes.get(global) {
            None => println!("nomi has no {}.{}", global.home.module, global.name),
            Some(n) if n != node => {
                let (a, b) = (format!("{n:?}"), format!("{node:?}"));
                let at = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
                let from = at.saturating_sub(150);
                let end = |s: &str| s.len().min(at + 250);
                println!(
                    "{} {}.{} differs at {at}:\n  nomi: …{}\n  elm:  …{}",
                    global.home.package,
                    global.home.module,
                    global.name,
                    &a[from..end(&a)],
                    &b[from..end(&b)]
                );
            }
            Some(_) => {}
        }
    }
    for global in ours.nodes.keys().filter(|g| !theirs.nodes.contains_key(*g)) {
        println!("elm has no {}.{}", global.home.module, global.name);
    }
    if ours.fields != theirs.fields {
        println!(
            "fields:\n  nomi: {:?}\n  elm:  {:?}",
            ours.fields, theirs.fields
        );
    }
    Ok(())
}
