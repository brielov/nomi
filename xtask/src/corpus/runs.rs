//! Whether nomi makes of every corpus project what each elm made of it,
//! as `cargo xtask oracle corpus-docs`, `corpus-check`, `corpus-graphs`,
//! `corpus-objects` and `corpus-javascript` recorded: `cargo xtask corpus
//! docs` wants the same `docs.json` of every package, byte for byte, and
//! `cargo xtask corpus check` the silence of `nomi check` on every app and
//! package elm accepted; `cargo xtask corpus graphs` the same optimized
//! graph of every module elm built; `cargo xtask corpus objects` the same
//! graph of the dependencies as `o.dat` holds; `cargo xtask corpus
//! javascript` the same JavaScript of every app in each mode; and all the
//! same `--report=json` wherever elm rejected the project. Each project's
//! dependencies are solved and downloaded as `corpus names` does, so this
//! runs by hand, never in CI. A project elm couldn't build for want of a
//! release the cache lacked is skipped.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use nomi_base::Compat;
use nomi_compiler::optimized::binary;
use nomi_engine::check::{self, Built, Checked, Goal, Made, Optimized, Style};
use nomi_engine::console::Captured;
use nomi_engine::graph::Env;
use nomi_engine::{javascript, names, objects};
use nomi_project::{ModuleName, Name};
use nomi_registry::Bundle;
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{parse_module, significant_tokens};
use nomi_testkit::{release_column, sha256_hex as sha256};

use super::compat;
use super::names::{Expected, Live, Project, Ready, build, in_chunks, solve};
use crate::oracle::corpus_runs::{Cell, Projects, Suite, graphs_cell, sandbox_id};
use crate::oracle::javascript::Mode;

pub(crate) fn check(root: &Path, suite: Suite, only: Option<&str>) -> Result<()> {
    let oracle = load(root, suite)?;
    let classification = compat::load(root)?;
    let live = Live::new()?;
    let mut matching = 0;
    let mut skipped: Vec<String> = Vec::new();
    let mut failures: Vec<(String, String)> = Vec::new();
    for &c in &classification.compats {
        let cells = (oracle.get(&c)).with_context(|| format!("{} has no elm {c}", suite.file()))?;
        let solving = solve(root, &live, &classification, c, only, suite.projects())?;
        // Dependencies without a solution stop elm before any module too.
        for (case, why) in solving.failures {
            if let Some(Cell::Error(_)) = cells.get(oracle_case(&case)) {
                matching += 1;
            } else {
                failures.push((case, why));
            }
        }
        skipped.extend(solving.unavailable);
        let mut ready = Vec::new();
        for (project, sources) in &solving.solved {
            let Some(expected) = cells.get(oracle_case(&project.case)) else {
                failures.push((project.case.clone(), "not in the oracle".to_owned()));
                continue;
            };
            if let Cell::Excluded | Cell::Error(_) | Cell::Crash = expected {
                skipped.push(format!("{}: elm said {expected}", project.case));
                continue;
            }
            ready.push(Ready {
                project,
                sources,
                given: expected,
            });
        }
        let made = |project: &Project, sources: &[_], interfaces: &_, expected: &&Cell| {
            run(suite, c, project, sources, interfaces, expected)
        };
        // Small chunks: each project holds its dependencies' interfaces
        // until its chunk is checked.
        in_chunks(c, &ready, 16, made, |outcomes| {
            for (project, outcome) in outcomes {
                match outcome {
                    Ok(()) => matching += 1,
                    Err(why) => failures.push((project.case.clone(), why)),
                }
            }
            eprintln!("  {c}: {matching} matching, {} failing", failures.len());
        });
    }
    for why in &skipped {
        eprintln!("skipped: {why}");
    }
    // Runs where elm's own output is wrong, each checked by hand, by case.
    let expected = Expected::load(root, &format!("corpus/{}/expected.tsv", suite.name()))?;
    let known = expected.report(&failures, only.is_none());
    let differ = failures.len() - known;
    let kind = match suite.projects() {
        Projects::All => "projects",
        Projects::Apps => "apps",
        Projects::Packages => "packages",
    };
    println!(
        "{}: {matching} {kind} made as elm makes them, {known} where elm is wrong, {differ} not, {} skipped",
        suite.name(),
        skipped.len()
    );
    if differ > 0 {
        bail!("{differ} {kind} differ");
    }
    Ok(())
}

/// Writes nomi's `bytes` as `file` in the sandbox where `suite`'s oracle
/// kept elm's, under `nomi/` beside its `out/`, where elm's differ, for
/// a look: `graph-diff` looks into graphs. Whether they differ.
fn kept(suite: Suite, compat: Compat, file: &str, bytes: &[u8]) -> bool {
    let Ok(dir) = suite.sandbox(compat) else {
        return false;
    };
    if fs::read(dir.join("out").join(file)).ok().as_deref() == Some(bytes) {
        return false;
    }
    let path = dir.join("nomi").join(file);
    let _ = fs::create_dir_all(path.parent().expect("a file in a directory"));
    let _ = fs::write(path, bytes);
    true
}

/// Which of `modules` differ from the `.elmo` files the oracle kept for
/// `case`, if it kept them.
fn differing(compat: Compat, case: &str, modules: &[(String, Vec<u8>)]) -> Option<String> {
    let id = sandbox_id(case);
    let elm = Suite::Graphs.sandbox(compat).ok()?.join("out").join(&id);
    if !elm.is_dir() {
        return None;
    }
    let differ: Vec<&str> = (modules.iter())
        .filter(|(name, bytes)| {
            let file = format!("{id}/{}.elmo", name.replace('.', "-"));
            kept(Suite::Graphs, compat, &file, bytes)
        })
        .map(|(name, _)| name.as_str())
        .collect();
    Some(differ.join(", "))
}

thread_local! {
    /// Each package's graph as each release makes it, made once a thread:
    /// projects share them.
    static OBJECTS: RefCell<HashMap<Compat, objects::Memo>> = Default::default();
}

/// The dependencies `sources` as `compat` builds them, from the thread's
/// [`OBJECTS`].
fn dependencies(
    compat: Compat,
    sources: &[(Name, PathBuf)],
) -> Result<objects::Dependencies, String> {
    OBJECTS.with(|memos| {
        let mut memos = memos.borrow_mut();
        objects::build(
            compat,
            sources,
            &Bundle::default(),
            memos.entry(compat).or_default(),
            None,
        )
    })
}

/// The oracle's cells: a column for each elm, by project case.
pub(super) fn load(root: &Path, suite: Suite) -> Result<BTreeMap<Compat, BTreeMap<String, Cell>>> {
    let path = suite.file();
    let text = fs::read_to_string(root.join(&path))
        .with_context(|| format!("reading {path}; run its oracle"))?;
    let mut lines = text.lines().filter(|l| !l.starts_with('#'));
    let header: Vec<&str> = lines
        .next()
        .context("an empty oracle")?
        .split('\t')
        .collect();
    let [_, elms @ ..] = &header[..] else {
        bail!("{path} has no header");
    };
    let compats = (elms.iter())
        .map(|elm| release_column(elm).with_context(|| format!("{path}'s column `{elm}`")))
        .collect::<Result<Vec<Compat>>>()?;
    let mut columns: BTreeMap<Compat, BTreeMap<String, Cell>> = BTreeMap::new();
    for line in lines {
        let row: Vec<&str> = line.split('\t').collect();
        ensure!(row.len() == header.len(), "{path}: malformed row `{line}`");
        for (&compat, cell) in compats.iter().zip(&row[1..]) {
            let column = columns.entry(compat).or_default();
            let fresh = column.insert(row[0].to_owned(), cell.parse()?).is_none();
            ensure!(fresh, "{path} has {} twice", row[0]);
        }
    }
    Ok(columns)
}

/// The oracle's case, `author/project@version` or `app/name`, of a case
/// `… on 0.19.x`.
fn oracle_case(case: &str) -> &str {
    case.split_once(" on ").map_or(case, |(case, _)| case)
}

/// Whether nomi makes what elm made of `project`, `expected`, as `suite`'s
/// `elm make`; why not where it doesn't.
fn run(
    suite: Suite,
    compat: Compat,
    project: &Project,
    sources: &[(Name, PathBuf)],
    interfaces: &names::PackageInterfaces,
    expected: &Cell,
) -> Result<(), String> {
    let goal = match suite {
        Suite::Docs => Goal::Docs,
        Suite::Check | Suite::Objects => Goal::Check,
        Suite::Graphs | Suite::JavaScript => Goal::Optimize,
    };
    let (built, env) = build(compat, project, sources, interfaces, goal)?;
    let Built {
        checked,
        made: output,
        ..
    } = built;
    let case = oracle_case(&project.case);
    let mut differ = None;
    let made = match (suite, output) {
        (Suite::Docs, Some(Made::Docs(docs))) => Some(Cell::Docs(sha256(docs.as_bytes()))),
        _ if !matches!(checked, Checked::Fine) => None,
        (Suite::Check, _) => Some(Cell::Ok),
        (Suite::Objects, _) => {
            let graph = dependencies(compat, sources)?.graph();
            let bytes = binary::encode_global_graph(&graph, compat);
            kept(
                Suite::Objects,
                compat,
                &format!("{}.o.dat", sandbox_id(case)),
                &bytes,
            );
            Some(Cell::Objects(sha256(&bytes)))
        }
        (Suite::JavaScript, Some(Made::Optimized(optimized))) => {
            Some(scripts(compat, case, sources, &optimized)?)
        }
        (Suite::Graphs, Some(Made::Optimized(optimized))) => {
            let modules = graphs(compat, project, &env, optimized);
            differ = differing(compat, case, &modules).filter(|why| !why.is_empty());
            Some(graphs_cell(&modules))
        }
        (Suite::Docs | Suite::JavaScript | Suite::Graphs, _) => {
            unreachable!("a clean build makes what its goal asks")
        }
    };
    let cell = match made {
        Some(cell) => cell,
        None => reported(project, &checked, compat)?,
    };
    if cell == *expected {
        return Ok(());
    }
    let differ = differ.map_or_else(String::new, |why| format!(" ({why})"));
    Err(format!("nomi: {cell}{differ}\n  elm:  {expected}"))
}

/// The cell of the report `nomi check` gives on `project` as elm, with
/// the project's directory written `$ROOT`.
fn reported(project: &Project, checked: &Checked, compat: Compat) -> Result<Cell, String> {
    let mut console = Captured::default();
    check::show(&mut console, &project.root, checked, compat, Style::Json);
    let root = project.root.to_str().ok_or("a path that isn't UTF-8")?;
    Ok(Cell::of_report(
        &console.err.trim_end().replace(root, "$ROOT"),
    ))
}

/// The JavaScript cell of an app nomi checked as `compat`, optimized as
/// `optimized`, with `sources` its dependencies'.
fn scripts(
    compat: Compat,
    case: &str,
    sources: &[(Name, PathBuf)],
    optimized: &Optimized,
) -> Result<Cell, String> {
    let built = dependencies(compat, sources)?;
    let refused = !javascript::debug_uses(compat, optimized).is_empty();
    let dependencies = (built.graph(), built.types());
    let hashes = Mode::ALL.map(|mode| {
        let kind = match mode {
            Mode::Dev => javascript::Kind::Dev,
            Mode::Debug => javascript::Kind::Debug,
            Mode::Optimize if refused => return None,
            Mode::Optimize => javascript::Kind::Optimize,
        };
        let program = javascript::Program {
            compat,
            kind,
            target: javascript::Target::default(),
            package: "author/project",
            packages: sources,
        };
        let js = javascript::program(program, dependencies.clone(), optimized.clone())
            .expect("the browser lacks no kernel of elm's");
        let file = format!("{}.{}.js", sandbox_id(case), mode.name());
        kept(Suite::JavaScript, compat, &file, &js);
        Some(sha256(&js))
    });
    Ok(Cell::JavaScript(hashes))
}

/// The graph of each module nomi optimized of `project`, built in `env`,
/// that elm writes an `.elmo` for: each module's name and its encoding.
fn graphs(
    compat: Compat,
    project: &Project,
    env: &Env,
    optimized: Optimized,
) -> Vec<(String, Vec<u8>)> {
    // elm writes no `.elmo` for a root outside the source directories,
    // which it compiles but doesn't keep.
    let outside: Vec<ModuleName> = (project.paths.iter())
        .map(|p| project.root.join(p))
        .filter(|p| !env.src_dirs().iter().any(|d| p.starts_with(d)))
        .filter_map(|p| fs::read_to_string(p).ok())
        .map(|text| root_name(&text))
        .collect();
    (optimized.graphs.into_iter())
        .filter(|(name, _)| !outside.contains(name))
        .map(|(name, graph)| (name.to_string(), binary::encode_local_graph(&graph, compat)))
        .collect()
}

/// The name a root module gives itself, or `Main` without a header, as
/// the engine names it.
fn root_name(text: &str) -> ModuleName {
    let module =
        ast::Module::cast(parse_module(text).syntax()).expect("a module parses to a module");
    match module.module_header().and_then(|h| h.module_name()) {
        Some(name) => ModuleName::new(
            &significant_tokens(name.syntax())
                .map(|t| t.text().to_owned())
                .collect::<String>(),
        ),
        None => ModuleName::new("Main"),
    }
}

/// `cargo xtask corpus graph-diff`: where the graph in `nomi` first
/// differs from the one in `elm`, both `.elmo` files of `compat`.
pub(crate) fn graph_diff(compat: Compat, nomi: &Path, elm: &Path) -> Result<()> {
    // An `o.dat` is a whole graph, read here as a module's without main.
    let read = |path: &Path| -> Result<nomi_compiler::optimized::LocalGraph> {
        let bytes = fs::read(path)?;
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
                let (from, to) = (at.saturating_sub(150), at + 250);
                println!(
                    "{} {}.{} differs at {at}:\n  nomi: …{}\n  elm:  …{}",
                    global.home.package,
                    global.home.module,
                    global.name,
                    window(&a, from, to),
                    window(&b, from, to)
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

/// `text` from byte `from` to byte `to`, each moved back to the start of
/// the character it falls in.
fn window(text: &str, from: usize, to: usize) -> &str {
    let start = |at: usize| {
        (0..=at.min(text.len()))
            .rev()
            .find(|&i| text.is_char_boundary(i))
            .unwrap_or(0)
    };
    &text[start(from)..start(to)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_end_between_characters() {
        assert_eq!(window("abcdef", 1, 4), "bcd");
        assert_eq!(window("aéb", 2, 3), "é");
        assert_eq!(window("aéb", 0, 2), "a");
        assert_eq!(window("aéb", 0, 99), "aéb");
    }

    #[test]
    fn cases_lose_their_release() {
        assert_eq!(oracle_case("app/todomvc on 0.19.1"), "app/todomvc");
        assert_eq!(oracle_case("elm/url@1.0.0"), "elm/url@1.0.0");
    }

    #[test]
    fn the_oracles_cells_read_back_as_written() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        for suite in [
            Suite::Docs,
            Suite::Check,
            Suite::Graphs,
            Suite::Objects,
            Suite::JavaScript,
        ] {
            let text = fs::read_to_string(root.join(suite.file())).unwrap();
            let columns = load(root, suite).unwrap();
            for line in text.lines().filter(|l| !l.starts_with('#')).skip(1) {
                let (case, cells) = line.split_once('\t').unwrap();
                let written: Vec<String> = (columns.values())
                    .map(|column| column[case].to_string())
                    .collect();
                assert_eq!(written.join("\t"), cells);
            }
        }
    }
}
