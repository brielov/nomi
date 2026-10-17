//! Running `nomi check` on a project an oracle recorded elm on, as the
//! oracle made it: in a directory whose path is as long as the oracle's,
//! with the case's files dated a second apart, an application's
//! `elm-version` set to the release under test, and the fixture packages.
//! Shared by the suites whose cases are projects, each test target taking
//! it as a public module, as each uses part of it.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use nomi_base::Compat;
use nomi_codegen_js::platform::Target;
use nomi_compiler::interface::Interface;
use nomi_compiler::optimized::binary;
use nomi_engine::Vfs;
use nomi_engine::check::{self, Build, Built, Checked, Goal, Made, Optimized, Style};
use nomi_engine::console::{Captured, Personality};
use nomi_engine::graph::{Env, Foreigns};
use nomi_engine::{javascript, names, objects};
use nomi_project::{ModuleName, Name, Outline};
use nomi_testkit::packages;
use nomi_testkit::transcript::Case;
use serde_json::Value;

/// What a cell holds where elm crashed; nothing to compare.
pub const CRASH: &str = "crash";

/// What a table calls the release `compat`.
pub fn compat_name(compat: Compat) -> String {
    format!("elm {compat}")
}

/// A table an oracle recorded, by case and elm.
pub fn table(path: &Path) -> BTreeMap<(String, String), String> {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}; run the oracle", path.display()));
    let mut out = BTreeMap::new();
    for line in text.lines().filter(|l| !l.starts_with('#')).skip(1) {
        let mut cells = line.splitn(3, '\t');
        let (case, elms, cell) = (
            cells.next().unwrap(),
            cells.next().unwrap(),
            cells.next().unwrap(),
        );
        for elm in elms.split(',') {
            out.insert((case.to_owned(), elm.to_owned()), cell.to_owned());
        }
    }
    out
}

/// What `nomi check` writes for a case.
#[derive(Debug)]
pub struct Reported {
    /// Standard error with `--report=json`.
    pub json: String,
    /// Standard error without.
    pub terminal: String,
    /// The `docs.json` it writes, if it writes one.
    pub docs: Option<String>,
}

/// What `nomi check` writes for `case` built as `goal` asks, with
/// `default_elm_json` where the case has no `elm.json`.
pub fn run(case: &Case, compat: Compat, default_elm_json: &str, goal: Goal) -> Reported {
    let project = Project::new(case, compat, default_elm_json);
    let paths = match goal {
        Goal::Docs => Vec::new(),
        Goal::Check | Goal::Optimize => paths(case),
    };
    let built = project.build(&paths, goal);
    let root_text = project.root.to_str().unwrap().to_owned();
    let [json, terminal] = [Style::Json, Style::Terminal].map(|style| {
        let mut console = Captured::default();
        check::show(&mut console, &project.root, &built.checked, compat, style);
        console.err.replace(&root_text, "$ROOT")
    });
    Reported {
        json,
        terminal,
        docs: match built.made {
            Some(Made::Docs(docs)) => Some(docs),
            _ => None,
        },
    }
}

/// What `elm make` optimizes of `case`, built as [`run`] builds it: each
/// module's graph, by name, as `compat` writes it to `<Module>.elmo`; or
/// why there is none.
pub fn optimized(
    case: &Case,
    compat: Compat,
    default_elm_json: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let (_, optimized) = optimize(case, compat, default_elm_json)?;
    Ok((optimized.graphs.into_iter())
        .map(|(name, graph)| (name.to_string(), binary::encode_local_graph(&graph, compat)))
        .collect())
}

/// The JavaScript `elm make` would generate for `case` as `kind`, built
/// as [`run`] builds it; or why there is none.
pub fn javascript(
    case: &Case,
    compat: Compat,
    kind: javascript::Kind,
    default_elm_json: &str,
) -> Result<Vec<u8>, String> {
    let (project, optimized) = optimize(case, compat, default_elm_json)?;
    let Outline::App(app) = &project.outline else {
        return Err("only applications generate JavaScript".to_owned());
    };
    let sources: Vec<(Name, PathBuf)> = (app.direct.iter())
        .chain(&app.indirect)
        .map(|(name, _)| (name.clone(), packages::release(&name.to_string()).dir()))
        .collect();
    let dependencies = dependencies(compat, &sources)?;
    let root = match optimized.roots.as_slice() {
        [root] => Some(root.name.to_string()),
        _ => None,
    };
    let js = javascript::program(
        compat,
        kind,
        Target::default(),
        None,
        "author/project",
        &sources,
        dependencies,
        optimized,
    )
    .map_err(|report| report.title.into_owned())?;
    let html = (case.args.iter()).any(|a| a.starts_with("--output=") && a.ends_with(".html"));
    match root {
        Some(module) if html => Ok(nomi_codegen_js::html::sandwich(&module, &js)),
        _ => Ok(js),
    }
}

/// `case`'s project, and what `elm make` optimizes of it; or the terminal
/// report on why it doesn't.
fn optimize(
    case: &Case,
    compat: Compat,
    default_elm_json: &str,
) -> Result<(Project, Optimized), String> {
    let project = Project::new(case, compat, default_elm_json);
    let built = project.build(&paths(case), Goal::Optimize);
    if !matches!(built.checked, Checked::Fine) {
        let mut console = Captured::default();
        check::show(
            &mut console,
            &project.root,
            &built.checked,
            compat,
            Style::Terminal,
        );
        return Err(console.err);
    }
    match built.made {
        Some(Made::Optimized(optimized)) => Ok((project, optimized)),
        _ => unreachable!("a clean build optimizes"),
    }
}

/// A case's project, made as the oracle made it, and what checking it
/// needs.
struct Project {
    _temp: tempfile::TempDir,
    root: PathBuf,
    outline: Outline,
    env: Env,
    interfaces: HashMap<String, Interface>,
}

impl Project {
    fn new(case: &Case, compat: Compat, default_elm_json: &str) -> Project {
        let temp = tempfile::tempdir().unwrap();
        let case = case.with_manifest(default_elm_json).as_release(compat);
        let root = case.lay_out_padded(temp.path()).unwrap();
        let (_, text) = case
            .files
            .iter()
            .find(|(path, _)| path == "elm.json")
            .unwrap();
        let outline = Outline::decode(text.as_bytes(), compat).expect("a good elm.json");
        let deps: Vec<Name> = outline.direct().cloned().collect();
        let foreigns = Foreigns::new(deps.iter().map(|dep| (dep.clone(), exposing(dep, compat))));
        let interfaces = names::visible(&interfaces(compat), &deps);
        let env = check::env_of(&root, &outline, compat, foreigns)
            .unwrap()
            .with_cwd(&root);
        Project {
            _temp: temp,
            root,
            outline,
            env,
            interfaces,
        }
    }

    /// The project built from `paths` as `goal` asks.
    fn build(&self, paths: &[PathBuf], goal: Goal) -> Built {
        let build = Build {
            snapshot: &Vfs::new().snapshot(),
            env: &self.env,
            outline: &self.outline,
            paths,
            personality: Personality::Elm,
            packages: (&self.interfaces).into(),
            artifacts: None,
        };
        build.run(goal).unwrap()
    }
}

/// The paths `elm make` is given among a case's arguments.
fn paths(case: &Case) -> Vec<PathBuf> {
    (case.args[1..].iter())
        .filter(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .collect()
}

/// The merged graph of `sources` as `compat` builds it, and their
/// modules' types, made once.
fn dependencies(
    compat: Compat,
    sources: &[(Name, PathBuf)],
) -> Result<
    (
        nomi_compiler::optimized::GlobalGraph,
        nomi_codegen_js::debug::Types,
    ),
    String,
> {
    static MEMOS: OnceLock<Mutex<HashMap<Compat, objects::Memo>>> = OnceLock::new();
    let mut memos = MEMOS.get_or_init(Default::default).lock().unwrap();
    let memo = memos.entry(compat).or_default();
    let built = objects::build(compat, sources, memo, None)?;
    Ok((built.graph(), built.types()))
}

/// Whether nomi's `--report=json` says what elm's cell does.
pub fn same_json(elm: &str, nomi: &str) -> bool {
    match elm {
        "accepted" => nomi.is_empty(),
        elm => {
            let elm: Value = serde_json::from_str(elm).unwrap();
            serde_json::from_str::<Value>(nomi).ok() == Some(elm)
        }
    }
}

/// The interfaces of every fixture package, as `compat` reads them, made
/// once per run.
pub fn interfaces(compat: Compat) -> Arc<names::PackageInterfaces> {
    static MADE: OnceLock<Mutex<HashMap<Compat, Arc<names::PackageInterfaces>>>> = OnceLock::new();
    let made = MADE.get_or_init(Default::default);
    if let Some(interfaces) = made.lock().unwrap().get(&compat) {
        return interfaces.clone();
    }
    let sources: Vec<_> = (packages::manifest().into_iter())
        .map(|package| (package.name.parse().unwrap(), package.dir()))
        .collect();
    let interfaces = Arc::new(names::package_interfaces(compat, &sources).unwrap());
    made.lock().unwrap().insert(compat, interfaces.clone());
    interfaces
}

/// The modules the fixture `dep`'s `elm.json` exposes.
pub fn exposing(dep: &Name, compat: Compat) -> Vec<ModuleName> {
    let text = fs::read(packages::release(&dep.to_string()).dir().join("elm.json")).unwrap();
    match Outline::decode(&text, compat).unwrap() {
        Outline::Pkg(pkg) => pkg.exposed.modules().cloned().collect(),
        Outline::App(_) => panic!("{dep} is an application"),
    }
}
