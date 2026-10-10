//! Running `nomi check` on a project an oracle recorded elm on, as the
//! oracle made it: in a directory whose path is as long as the oracle's,
//! with the case's files dated a second apart, and an application's
//! `elm-version` set to the release under test. Shared by the suites
//! whose cases are projects.
// Each test target uses only part of the support it shares.
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, UNIX_EPOCH};

use nomi_base::Compat;
use nomi_engine::Vfs;
use nomi_engine::check::{self, Style};
use nomi_engine::console::{Console, Personality};
use nomi_engine::graph::Foreigns;
use nomi_engine::names;
use nomi_engine::{javascript, objects};
use nomi_project::{ModuleName, Name, Outline};
use nomi_testkit::transcript::Case;
use serde_json::Value;

pub const ALL: [Compat; 3] = [Compat::V0_19_1, Compat::V0_19_2, Compat::V0_19_3];

/// What a cell holds where elm crashed; nothing to compare.
pub const CRASH: &str = "crash";

/// As long as the project directory the oracle ran elm in.
const ROOT_LENGTH: usize = 96;

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

/// What `nomi check` writes on standard error for `case`, with
/// `--report=json` and without, building with the packages stored under
/// `packages` (`<author>/<project>/<version>/elm.json`) and
/// `default_elm_json` where the case has no `elm.json`.
pub fn run(
    case: &Case,
    compat: Compat,
    default_elm_json: &str,
    packages: &Path,
) -> (String, String) {
    let (json, stderr, _) = build(case, compat, default_elm_json, packages, false);
    (json, stderr)
}

/// [`run`] as `elm make --docs` runs, and the `docs.json` it writes, if
/// it writes one.
pub fn run_docs(
    case: &Case,
    compat: Compat,
    default_elm_json: &str,
    packages: &Path,
) -> (String, String, Option<String>) {
    build(case, compat, default_elm_json, packages, true)
}

/// A case's project, made as the oracle made it, and what checking it
/// needs.
struct Project {
    _temp: tempfile::TempDir,
    root: PathBuf,
    outline: Outline,
    env: nomi_engine::graph::Env,
    interfaces: HashMap<String, nomi_compiler::interface::Interface>,
}

impl Project {
    fn new(case: &Case, compat: Compat, default_elm_json: &str, packages: &Path) -> Project {
        let temp = tempfile::tempdir().unwrap();
        let root = padded(temp.path());
        case.create_files(&root).unwrap();
        date_files(&root, case);
        let manifest = root.join("elm.json");
        let text = fs::read_to_string(&manifest).unwrap_or_else(|_| default_elm_json.to_owned());
        let text = with_elm_version(&text, compat);
        fs::write(&manifest, &text).unwrap();
        let outline = Outline::decode(text.as_bytes(), compat).expect("a good elm.json");
        let deps: Vec<Name> = match &outline {
            Outline::App(app) => app.direct.keys().cloned().collect(),
            Outline::Pkg(pkg) => pkg.deps.keys().cloned().collect(),
        };
        let foreigns = Foreigns::new(
            deps.iter()
                .map(|dep| (dep.clone(), exposing(packages, dep, compat))),
        );
        let interfaces = names::visible(&interfaces(packages, compat), &deps);
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
}

/// The paths `elm make` is given among a case's arguments.
fn paths(case: &Case) -> Vec<PathBuf> {
    (case.args[1..].iter())
        .filter(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .collect()
}

/// What `elm make` optimizes of `case`, built as [`run`] builds it: each
/// module's graph, by name, as `compat` writes it to `<Module>.elmo`; or
/// why there is none.
pub fn optimized(
    case: &Case,
    compat: Compat,
    default_elm_json: &str,
    packages: &Path,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let project = Project::new(case, compat, default_elm_json, packages);
    let (checked, optimized) = check::optimize(
        &Vfs::new().snapshot(),
        &project.env,
        &project.outline,
        &paths(case),
        Personality::Elm,
        &project.interfaces,
    );
    if !matches!(checked, check::Checked::Fine) {
        let mut console = Captured::default();
        check::show(
            &mut console,
            &project.root,
            &checked,
            compat,
            Style::Terminal,
        );
        return Err(console.err);
    }
    Ok((optimized.graphs.into_iter())
        .map(|(name, graph)| {
            let bytes = nomi_compiler::optimized::binary::encode_local_graph(&graph, compat);
            (name.to_string(), bytes)
        })
        .collect())
}

/// The JavaScript `elm make` would generate for `case` in `mode` (`dev`,
/// `debug` or `optimize`), built as [`run`] builds it; or why there is
/// none.
pub fn javascript(
    case: &Case,
    compat: Compat,
    mode: &str,
    default_elm_json: &str,
    packages: &Path,
) -> Result<Vec<u8>, String> {
    let project = Project::new(case, compat, default_elm_json, packages);
    let (checked, optimized) = check::optimize(
        &Vfs::new().snapshot(),
        &project.env,
        &project.outline,
        &paths(case),
        Personality::Elm,
        &project.interfaces,
    );
    if !matches!(checked, check::Checked::Fine) {
        let mut console = Captured::default();
        check::show(
            &mut console,
            &project.root,
            &checked,
            compat,
            Style::Terminal,
        );
        return Err(console.err);
    }
    let Outline::App(app) = &project.outline else {
        return Err("only applications generate JavaScript".to_owned());
    };
    let sources: Vec<(Name, PathBuf)> = (app.direct.iter())
        .chain(&app.indirect)
        .map(|(name, version)| {
            let dir = packages.join(name.to_string()).join(version.to_string());
            (name.clone(), dir)
        })
        .collect();
    let dependencies = dependencies(compat, &sources)?;
    let kind = match mode {
        "dev" => javascript::Kind::Dev,
        "debug" => javascript::Kind::Debug,
        _ => javascript::Kind::Optimize,
    };
    let roots = optimized.roots.clone();
    let js = javascript::program(compat, kind, "author/project", dependencies, optimized);
    let html = (case.args.iter()).any(|a| a.starts_with("--output=") && a.ends_with(".html"));
    match roots.as_slice() {
        [(module, _)] if html => Ok(nomi_codegen_js::html::sandwich(&module.to_string(), &js)),
        _ => Ok(js),
    }
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

fn build(
    case: &Case,
    compat: Compat,
    default_elm_json: &str,
    packages: &Path,
    docs: bool,
) -> (String, String, Option<String>) {
    let project = Project::new(case, compat, default_elm_json, packages);
    let (root, outline, env, interfaces) = (
        &project.root,
        &project.outline,
        &project.env,
        &project.interfaces,
    );
    let snapshot = Vfs::new().snapshot();
    let (checked, written) = if docs {
        check::docs(&snapshot, env, outline, Personality::Elm, interfaces)
    } else {
        let checked = check::check(
            &snapshot,
            env,
            outline,
            &paths(case),
            Personality::Elm,
            interfaces,
        );
        (checked, None)
    };
    let root_text = root.to_str().unwrap().to_owned();
    let [json, stderr] = [Style::Json, Style::Terminal].map(|style| {
        let mut console = Captured::default();
        check::show(&mut console, root, &checked, compat, style);
        console.err.replace(&root_text, "$ROOT")
    });
    (json, stderr, written)
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

/// A new directory in `base` whose canonical path is [`ROOT_LENGTH`]
/// bytes long, as the oracle's was.
fn padded(base: &Path) -> PathBuf {
    let base = fs::canonicalize(base).unwrap();
    let len = base.to_str().unwrap().len();
    let dir = base.join("p".repeat(ROOT_LENGTH - len - 1));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Dates the case's files a second apart in their order, as the oracle
/// did.
fn date_files(root: &Path, case: &Case) {
    let start = UNIX_EPOCH + Duration::from_secs(978_307_200);
    for (i, (path, _)) in case.files.iter().enumerate() {
        let file = fs::File::options()
            .write(true)
            .open(root.join(path))
            .unwrap();
        file.set_modified(start + Duration::from_secs(i as u64))
            .unwrap();
    }
}

fn with_elm_version(text: &str, compat: Compat) -> String {
    const KEY: &str = "\"elm-version\": \"";
    if !text.contains("\"type\": \"application\"") {
        return text.to_owned();
    }
    let start = text.find(KEY).unwrap() + KEY.len();
    let end = start + text[start..].find('"').unwrap();
    format!("{}{compat}{}", &text[..start], &text[end..])
}

/// The interfaces of every package stored under `packages`, as `compat`
/// reads them, made once per run.
pub fn interfaces(packages: &Path, compat: Compat) -> Arc<names::PackageInterfaces> {
    type Made = HashMap<(PathBuf, Compat), Arc<names::PackageInterfaces>>;
    static MADE: OnceLock<Mutex<Made>> = OnceLock::new();
    let made = MADE.get_or_init(Default::default);
    let key = (packages.to_owned(), compat);
    if let Some(interfaces) = made.lock().unwrap().get(&key) {
        return interfaces.clone();
    }
    let mut sources = Vec::new();
    for author in fs::read_dir(packages).unwrap() {
        let author = author.unwrap().path();
        if !author.is_dir() {
            continue;
        }
        for project in fs::read_dir(&author).unwrap() {
            let project = project.unwrap().path();
            let version = fs::read_dir(&project)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let name = format!(
                "{}/{}",
                author.file_name().unwrap().to_string_lossy(),
                project.file_name().unwrap().to_string_lossy()
            );
            let name = Name::parse(
                name.as_bytes(),
                nomi_project::json::Cursor::start(compat),
                compat,
            )
            .unwrap();
            sources.push((name, version));
        }
    }
    let interfaces = Arc::new(names::package_interfaces(compat, &sources).unwrap());
    made.lock().unwrap().insert(key, interfaces.clone());
    interfaces
}

/// The modules `dep`'s stored `elm.json` exposes.
pub fn exposing(packages: &Path, dep: &Name, compat: Compat) -> Vec<ModuleName> {
    let dir = packages.join(dep.to_string());
    let version = fs::read_dir(&dir)
        .unwrap_or_else(|_| panic!("{dep} isn't stored"))
        .next()
        .unwrap()
        .unwrap()
        .path();
    let text = fs::read(version.join("elm.json")).unwrap();
    match Outline::decode(&text, compat).unwrap() {
        Outline::Pkg(pkg) => pkg.exposed.modules().cloned().collect(),
        Outline::App(_) => panic!("{dep} is an application"),
    }
}

/// A console that keeps what is written to standard error, as a pipe
/// would.
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
