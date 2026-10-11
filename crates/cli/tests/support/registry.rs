//! The registry transcripts' runner (plan 8.14), shared by the `elm_diff`
//! and `elm_publish` ratchets. The commands insist on the latest
//! registry, so the cases run against one served on the loopback
//! interface (`NOMI_REGISTRY`): every package name elm's registry had,
//! the releases of the packages the cases read, their documentation as
//! the elm emulated read it, and the packages vendored for the
//! compiler's tests to build with. As in the recordings, git finds no
//! repository.
//!
//! An application whose `elm-version` names a pinned elm runs as each; any
//! other case as the newest, as nomi builds a package. Where that elm
//! crashed or hung, as 0.19.3 does diffing a local package, the case is
//! checked against the other elms' transcript when they agree.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::server::{self, Server};
use super::website::{Website, offline};
use nomi_registry::Http;
use nomi_testkit::ratchet::Suite;
use nomi_testkit::transcript::Case;

const RELEASES: [&str; 3] = ["0.19.1", "0.19.2", "0.19.3"];

/// The length of the recordings' project directory.
const ROOT_LENGTH: usize = 96;

/// Runs the cases under `cases`, against their transcripts, and settles
/// the ratchet there.
pub fn transcripts(cases: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(cases);
    let bin = tempfile::tempdir().unwrap();
    let elm = bin.path().join("elm");
    fs::copy(env!("CARGO_BIN_EXE_nomi"), &elm).unwrap();
    let server = Server::start(registry(&dir, "0.19.3"));
    let mut outcomes = Vec::new();
    let mut paths: Vec<PathBuf> = fs::read_dir(dir.join("cases"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    paths.sort();
    for path in &paths {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let case = Case::parse(&fs::read_to_string(path).unwrap()).unwrap();
        let expected =
            fs::read_to_string(dir.join("expected").join(format!("{name}.txt"))).unwrap();
        let releases: &[&str] = match release_named(&case) {
            Some(_) => &RELEASES,
            None => &RELEASES[2..],
        };
        for release in releases {
            let id = format!("{name} as {release}");
            let Some(expected) = transcript(&expected, release) else {
                continue;
            };
            let case = as_release(&case, release);
            let cache = tempfile::tempdir().unwrap();
            let outcome = run(&elm, cache.path(), &server.url, &case, expected);
            if let (Err(why), Ok(_)) = (&outcome, std::env::var("NOMI_SHOW")) {
                eprintln!("FAIL {id}\n{why}");
            }
            outcomes.push((id, outcome));
        }
    }
    Suite::open(&dir).finish(outcomes);
}

/// The registry the recordings had, as `release` read it.
pub fn registry(dir: &Path, release: &str) -> Arc<server::Handler> {
    let packages = Path::new(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/packages");
    let website = Website::with_sources(&packages);
    let mut versions: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in fs::read_to_string(dir.join("packages.txt"))
        .unwrap()
        .lines()
    {
        versions.insert(name.to_owned(), vec!["1.0.0".to_owned()]);
    }
    let releases = dir.join("releases");
    for author in fs::read_dir(&releases).unwrap() {
        let author = author.unwrap().path();
        for project in fs::read_dir(&author).unwrap() {
            let project = project.unwrap().path();
            let name = format!(
                "{}/{}",
                author.file_name().unwrap().to_string_lossy(),
                project.file_stem().unwrap().to_string_lossy()
            );
            let json: serde_json::Value =
                serde_json::from_slice(&fs::read(&project).unwrap()).unwrap();
            let listed = json.as_object().unwrap().keys().cloned().collect();
            versions.insert(name, listed);
        }
    }
    let all: Vec<String> = (versions.iter())
        .map(|(name, vs)| {
            let vs: Vec<String> = vs.iter().map(|v| format!("\"{v}\"")).collect();
            format!("\"{name}\":[{}]", vs.join(","))
        })
        .collect();
    let all = format!("{{{}}}", all.join(","));
    let docs = dir.join("docs").join(release);
    Arc::new(
        move |method: &str, path: &str, _: &[u8]| match (method, path) {
            ("POST", "/all-packages") => Some(all.clone().into_bytes()),
            ("POST", p) if p.starts_with("/all-packages/since/") => Some(b"[]".to_vec()),
            ("GET", p) if p.ends_with("/docs.json") => {
                let file = p.strip_prefix("/packages/")?;
                fs::read(docs.join(file)).ok()
            }
            ("GET", p) if p.ends_with("/elm.json") => {
                fs::read(packages.join(p.strip_prefix("/packages/")?)).ok()
            }
            ("GET", p) => website
                .get(&format!("https://package.elm-lang.org{p}"))
                .ok(),
            _ => None,
        },
    )
}

/// nomi's transcript of `case`, against `expected`.
fn run(
    elm: &Path,
    cache: &Path,
    registry: &str,
    case: &Case,
    expected: &str,
) -> Result<(), String> {
    let work = tempfile::tempdir().unwrap();
    let root = padded(work.path());
    case.create_files(&root).unwrap();
    case.date_files(&root).unwrap();
    // Every request is for the registry, which `NOMI_REGISTRY` sends to
    // the loopback server, past the dead proxy `offline` names.
    let mut command = offline(elm, cache);
    command
        .env("NOMI_REGISTRY", registry)
        .env_remove("ALL_PROXY");
    let actual = case.run(command, &root).unwrap().compared(case, &root);
    if actual == expected {
        Ok(())
    } else {
        Err(format!("--- expected\n{expected}--- actual\n{actual}"))
    }
}

/// The transcript `release` recorded, or where it crashed or hung, the
/// one every other elm recorded alike.
fn transcript<'a>(expected: &'a str, release: &str) -> Option<&'a str> {
    let groups: Vec<(&str, &str)> = (expected.split("### ").skip(1))
        .filter_map(|group| group.split_once('\n'))
        .collect();
    let elm = format!("elm {release}");
    let (_, own) = groups
        .iter()
        .find(|(elms, _)| elms.split(", ").any(|e| e == elm))?;
    if !crashed(own) {
        return Some(own);
    }
    match groups
        .iter()
        .filter(|(_, t)| !crashed(t))
        .collect::<Vec<_>>()[..]
    {
        [(_, other)] => Some(other),
        _ => None,
    }
}

fn crashed(transcript: &str) -> bool {
    transcript.starts_with("=== killed")
        || transcript.contains("bypassed the normal error reporting process")
}

/// The pinned elm an application's `elm-version` names.
fn release_named(case: &Case) -> Option<&'static str> {
    let manifest = case.files.iter().find(|(path, _)| path == "elm.json")?;
    if !manifest.1.contains("\"type\": \"application\"") {
        return None;
    }
    (RELEASES.iter())
        .find(|r| manifest.1.contains(&format!("\"elm-version\": \"{r}\"")))
        .copied()
}

/// The case with its application's pinned `elm-version` set to `release`.
fn as_release(case: &Case, release: &str) -> Case {
    let mut case = case.clone();
    for (path, text) in &mut case.files {
        if path == "elm.json" && text.contains("\"type\": \"application\"") {
            for pinned in RELEASES {
                let from = format!("\"elm-version\": \"{pinned}\"");
                *text = text.replace(&from, &format!("\"elm-version\": \"{release}\""));
            }
        }
    }
    case
}

/// A new directory in `base` whose canonical path is [`ROOT_LENGTH`]
/// bytes long.
fn padded(base: &Path) -> PathBuf {
    let base = fs::canonicalize(base).unwrap();
    let pad = ROOT_LENGTH - base.as_os_str().len() - 1;
    let dir = base.join("p".repeat(pad));
    fs::create_dir_all(&dir).unwrap();
    dir
}
