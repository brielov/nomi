//! The reports on module graphs against each elm's, for the projects in
//! `tests/modules/cases` (`cargo xtask oracle modules` records them).
//!
//! A project problem must match elm's report whole. Otherwise each module
//! must have the import and module-name problems elm lists for it, and
//! no others; which modules elm shows, in what order, and their other
//! problems, are for the reporting policy and the phases after.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use nomi_base::Compat;
use nomi_base::diagnostic::json;
use nomi_engine::Vfs;
use nomi_engine::graph::{Env, Foreigns, Graph, Owner};
use nomi_engine::report;
use nomi_project::{ModuleName, Name, Outline};
use nomi_testkit::transcript::Case;
use serde_json::Value;

const ALL: [Compat; 3] = [Compat::V0_19_1, Compat::V0_19_2, Compat::V0_19_3];

/// The titles of the reports this suite covers inside modules.
const TITLES: [&str; 4] = [
    "MODULE NOT FOUND",
    "AMBIGUOUS IMPORT",
    "MODULE NAME MISSING",
    "MODULE NAME MISMATCH",
];

/// As long as the project directory the oracle ran elm in.
const ROOT_LENGTH: usize = 96;

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/modules")
}

#[test]
fn module_reports() {
    let expected = expected();
    let mut cases: Vec<String> = fs::read_dir(dir().join("cases"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    cases.sort();
    assert!(!cases.is_empty());
    let mut failures = Vec::new();
    for file in &cases {
        let stem = file.strip_suffix(".case").unwrap();
        let case = Case::parse(&fs::read_to_string(dir().join("cases").join(file)).unwrap())
            .unwrap_or_else(|e| panic!("{file}: {e}"));
        for compat in ALL {
            let elm = &expected[&(stem.to_owned(), compat_name(compat))];
            let got = run(&case, compat);
            if let Err(why) = compare(elm, &got) {
                failures.push(format!("{stem} ({}): {why}", compat_name(compat)));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

fn compat_name(compat: Compat) -> String {
    format!("elm {compat}")
}

/// `reports.tsv`, by case and elm.
fn expected() -> BTreeMap<(String, String), String> {
    let text =
        fs::read_to_string(dir().join("reports.tsv")).expect("run `cargo xtask oracle modules`");
    let mut out = BTreeMap::new();
    for line in text.lines().filter(|l| !l.starts_with('#')).skip(1) {
        let mut cells = line.splitn(3, '\t');
        let (case, elms, report) = (
            cells.next().unwrap(),
            cells.next().unwrap(),
            cells.next().unwrap(),
        );
        for elm in elms.split(',') {
            out.insert((case.to_owned(), elm.to_owned()), report.to_owned());
        }
    }
    out
}

/// What nomi makes of the case.
enum Outcome {
    Project(String),
    Modules(Vec<(String, String, Vec<String>)>),
}

fn run(case: &Case, compat: Compat) -> Outcome {
    let temp = tempfile::tempdir().unwrap();
    let root = padded(temp.path());
    case.create_files(&root).unwrap();
    let manifest = root.join("elm.json");
    let text = fs::read_to_string(&manifest)
        .unwrap_or_else(|_| fs::read_to_string(dir().join("default.elm.json")).unwrap());
    let text = with_elm_version(&text, compat);
    fs::write(&manifest, &text).unwrap();
    let outline = Outline::decode(text.as_bytes(), compat).expect("a good elm.json");
    let (owner, src_dirs, deps, exposed) = match &outline {
        Outline::App(app) => (
            Owner::Application,
            app.source_dirs
                .iter()
                .map(|d| PathBuf::from(String::from_utf8(d.raw().to_vec()).unwrap()))
                .collect(),
            app.direct.keys().cloned().collect::<Vec<_>>(),
            Vec::new(),
        ),
        Outline::Pkg(pkg) => (
            Owner::Package(pkg.name.clone()),
            vec![PathBuf::from("src")],
            pkg.deps.keys().cloned().collect(),
            pkg.exposed.modules().cloned().collect(),
        ),
    };
    let foreigns = Foreigns::new(deps.iter().map(|dep| (dep.clone(), exposing(dep, compat))));
    let env = Env::new(compat, &root, owner, &src_dirs, foreigns)
        .unwrap()
        .with_cwd(&root);
    let snapshot = Vfs::new().snapshot();
    let paths: Vec<PathBuf> = case.args[1..].iter().map(PathBuf::from).collect();
    let graph = if paths.is_empty() {
        Graph::from_exposed(&snapshot, &env, &exposed)
    } else {
        Graph::from_paths(&snapshot, &env, &paths)
    };
    let root = root.to_str().unwrap().to_owned();
    let unroot = |s: String| s.replace(&root, "$ROOT");
    let graph = match graph {
        Err(problem) => {
            let report = report::project_problem(&problem, compat);
            return Outcome::Project(unroot(json::report(&report, compat).to_string()));
        }
        Ok(graph) => graph,
    };
    if let Some(report) = report::missing_exposed(&graph, &exposed) {
        return Outcome::Project(unroot(json::report(&report, compat).to_string()));
    }
    let modules = report::module_errors(&env, &graph)
        .into_iter()
        .map(|m| {
            let problems = m
                .problems
                .iter()
                .map(|p| unroot(p.to_json(compat).to_string()))
                .collect();
            (
                unroot(m.path.to_string_lossy().into_owned()),
                m.name,
                problems,
            )
        })
        .collect();
    Outcome::Modules(modules)
}

fn compare(elm: &str, got: &Outcome) -> Result<(), String> {
    let elm_json: Option<Value> = (elm != "accepted").then(|| serde_json::from_str(elm).unwrap());
    let is_project = elm_json.as_ref().is_some_and(|v| v["type"] == "error");
    match got {
        Outcome::Project(report) => {
            let report: Value = serde_json::from_str(report).unwrap();
            if is_project && elm_json.as_ref() == Some(&report) {
                Ok(())
            } else {
                Err(format!("nomi:\n{report}\nelm:\n{elm}"))
            }
        }
        Outcome::Modules(modules) => {
            if is_project {
                return Err(format!("elm reports the project:\n{elm}"));
            }
            // By path: the name, and the problems this suite covers.
            let mut want: BTreeMap<String, (String, Vec<Value>)> = BTreeMap::new();
            if let Some(Value::Array(errors)) = elm_json.as_ref().map(|v| &v["errors"]) {
                for module in errors {
                    let problems: Vec<Value> = module["problems"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|p| TITLES.contains(&p["title"].as_str().unwrap()))
                        .cloned()
                        .collect();
                    if !problems.is_empty() {
                        let name = module["name"].as_str().unwrap().to_owned();
                        want.insert(
                            module["path"].as_str().unwrap().to_owned(),
                            (name, problems),
                        );
                    }
                }
            }
            let have: BTreeMap<String, (String, Vec<Value>)> = modules
                .iter()
                .map(|(path, name, problems)| {
                    let problems: Vec<Value> = problems
                        .iter()
                        .map(|p| serde_json::from_str::<Value>(p).unwrap())
                        .filter(|p| TITLES.contains(&p["title"].as_str().unwrap()))
                        .collect();
                    (path.clone(), (name.clone(), problems))
                })
                .filter(|(_, (_, problems))| !problems.is_empty())
                .collect();
            if want == have {
                Ok(())
            } else {
                Err(format!("nomi:\n{have:#?}\nelm:\n{want:#?}"))
            }
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

fn with_elm_version(text: &str, compat: Compat) -> String {
    const KEY: &str = "\"elm-version\": \"";
    if !text.contains("\"type\": \"application\"") {
        return text.to_owned();
    }
    let start = text.find(KEY).unwrap() + KEY.len();
    let end = start + text[start..].find('"').unwrap();
    format!("{}{compat}{}", &text[..start], &text[end..])
}

/// The modules `dep`'s stored `elm.json` exposes.
fn exposing(dep: &Name, compat: Compat) -> Vec<ModuleName> {
    let dir = dir().join("packages").join(dep.to_string());
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
