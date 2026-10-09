//! `nomi check` against `elm make --output=/dev/null`, for the projects
//! in `tests/modules/cases` (`cargo xtask oracle modules` records them):
//! the same `--report=json`, and the same terminal report on standard
//! error, from each elm. Every case fails, if it does, in the phases nomi
//! has: the project, syntax, module names and imports.
//!
//! The reports were recorded on Linux. On Windows elm draws cycles in
//! ASCII and separates paths with `\`, as nomi does there, so they hold
//! on Unix only.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use std::io;
use std::time::{Duration, UNIX_EPOCH};

use nomi_base::Compat;
use nomi_engine::Vfs;
use nomi_engine::check::{self, Style};
use nomi_engine::console::{Console, Personality};
use nomi_engine::graph::Foreigns;
use nomi_project::{ModuleName, Name, Outline};
use nomi_testkit::transcript::Case;
use serde_json::Value;

const ALL: [Compat; 3] = [Compat::V0_19_1, Compat::V0_19_2, Compat::V0_19_3];

/// As long as the project directory the oracle ran elm in.
const ROOT_LENGTH: usize = 96;

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/modules")
}

#[test]
fn module_reports() {
    let reports = table("reports.tsv");
    let terminal = table("terminal.tsv");
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
            let key = (stem.to_owned(), compat_name(compat));
            let (json, stderr) = run(&case, compat);
            let elm = &reports[&key];
            let same = match elm.as_str() {
                "accepted" => json.is_empty(),
                elm => {
                    let elm: Value = serde_json::from_str(elm).unwrap();
                    serde_json::from_str::<Value>(&json).ok() == Some(elm)
                }
            };
            if !same {
                failures.push(format!(
                    "{stem} ({compat}) --report=json:\nnomi:\n{json}\nelm:\n{elm}"
                ));
            }
            let elm: String = serde_json::from_str(&terminal[&key]).unwrap();
            if stderr != elm {
                failures.push(format!(
                    "{stem} ({compat}) terminal:\nnomi:\n{stderr}\nelm:\n{elm}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

fn compat_name(compat: Compat) -> String {
    format!("elm {compat}")
}

/// A table the oracle recorded, by case and elm.
fn table(file: &str) -> BTreeMap<(String, String), String> {
    let text = fs::read_to_string(dir().join(file)).expect("run `cargo xtask oracle modules`");
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

/// What `nomi check` writes on standard error for the case, with
/// `--report=json` and without.
fn run(case: &Case, compat: Compat) -> (String, String) {
    let temp = tempfile::tempdir().unwrap();
    let root = padded(temp.path());
    case.create_files(&root).unwrap();
    date_files(&root, case);
    let manifest = root.join("elm.json");
    let text = fs::read_to_string(&manifest)
        .unwrap_or_else(|_| fs::read_to_string(dir().join("default.elm.json")).unwrap());
    let text = with_elm_version(&text, compat);
    fs::write(&manifest, &text).unwrap();
    let outline = Outline::decode(text.as_bytes(), compat).expect("a good elm.json");
    let deps: Vec<Name> = match &outline {
        Outline::App(app) => app.direct.keys().cloned().collect(),
        Outline::Pkg(pkg) => pkg.deps.keys().cloned().collect(),
    };
    let foreigns = Foreigns::new(deps.iter().map(|dep| (dep.clone(), exposing(dep, compat))));
    let env = check::env_of(&root, &outline, compat, foreigns)
        .unwrap()
        .with_cwd(&root);
    let paths: Vec<PathBuf> = case.args[1..].iter().map(PathBuf::from).collect();
    let checked = check::check(
        &Vfs::new().snapshot(),
        &env,
        &outline,
        &paths,
        Personality::Elm,
    );
    let root_text = root.to_str().unwrap().to_owned();
    let mut outputs = [Style::Json, Style::Terminal].map(|style| {
        let mut console = Captured::default();
        check::show(&mut console, &root, &checked, compat, style);
        console.err.replace(&root_text, "$ROOT")
    });
    (
        std::mem::take(&mut outputs[0]),
        std::mem::take(&mut outputs[1]),
    )
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

/// A console that keeps what is written to it, as a pipe would.
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
