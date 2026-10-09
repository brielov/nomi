//! Running `nomi check` on a project an oracle recorded elm on, as the
//! oracle made it: in a directory whose path is as long as the oracle's,
//! with the case's files dated a second apart, and an application's
//! `elm-version` set to the release under test. Shared by the suites
//! whose cases are projects.
// Each test target uses only part of the support it shares.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use nomi_base::Compat;
use nomi_engine::Vfs;
use nomi_engine::check::{self, Style};
use nomi_engine::console::{Console, Personality};
use nomi_engine::graph::Foreigns;
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
    let [json, stderr] = [Style::Json, Style::Terminal].map(|style| {
        let mut console = Captured::default();
        check::show(&mut console, &root, &checked, compat, style);
        console.err.replace(&root_text, "$ROOT")
    });
    (json, stderr)
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

/// The modules `dep`'s stored `elm.json` exposes.
fn exposing(packages: &Path, dep: &Name, compat: Compat) -> Vec<ModuleName> {
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
