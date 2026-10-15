//! The registry transcripts' runner, shared by the `elm_diff`
//! and `elm_publish` ratchets. The commands insist on the latest
//! registry, so the cases run against one served on the loopback
//! interface (`NOMI_REGISTRY`): every package name elm's registry had,
//! the releases of the packages the cases read, their documentation as
//! the newest elm read it, and the packages vendored for the compiler's
//! tests to build with. As in the recordings, git finds no
//! repository.
//!
//! An application whose `elm-version` names a pinned elm runs as each; any
//! other case as the newest, as nomi builds a package. Where that elm
//! crashed or hung, as 0.19.3 does diffing a local package, the case is
//! checked against the other elms' transcript when they agree.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use nomi_base::Compat;
use nomi_registry::Http;
use nomi_testkit::packages;
use nomi_testkit::ratchet::{Suite, shown};
use nomi_testkit::transcript::{self, crashed, recorded};

use super::server::{self, Method, Server};
use super::website::{self, Website, offline};

/// Runs the cases under `cases`, against their transcripts, and settles
/// the ratchet there.
pub fn transcripts(cases: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(cases);
    let (_bin, elm) = website::personality("elm");
    let server = Server::start(registry(&dir));
    let mut outcomes = Vec::new();
    for (name, case) in transcript::cases(&dir.join("cases")) {
        let expected =
            fs::read_to_string(dir.join("expected").join(format!("{name}.txt"))).unwrap();
        for &release in case.releases() {
            let id = format!("{name} as {release}");
            let Some(expected) = transcript(&expected, release) else {
                continue;
            };
            let cache = tempfile::tempdir().unwrap();
            // Every request is for the registry, which `NOMI_REGISTRY`
            // sends to the loopback server, past the dead proxy `offline`
            // names.
            let mut command = offline(&elm, cache.path());
            command
                .env("NOMI_REGISTRY", &server.url)
                .env_remove("ALL_PROXY");
            let actual = website::transcript(&case.clone().as_release(release), command);
            outcomes.push(shown(id, transcript::matching(expected, &actual)));
        }
    }
    Suite::open(&dir).finish(outcomes);
}

/// The registry the recordings had, as the newest elm read it.
pub fn registry(dir: &Path) -> Arc<server::Handler> {
    let website = Website::with_sources(&packages::dir());
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
    let all = serde_json::to_vec(&versions).unwrap();
    let docs = dir.join("docs").join(Compat::LATEST.as_str());
    Arc::new(
        move |method: Method, path: &str, _: &[u8]| match (method, path) {
            (Method::Post, "/all-packages") => Some(all.clone()),
            (Method::Post, p) if p.starts_with("/all-packages/since/") => Some(b"[]".to_vec()),
            (Method::Get, p) if p.ends_with("/docs.json") => {
                let file = p.strip_prefix("/packages/")?;
                fs::read(docs.join(file)).ok()
            }
            (Method::Get, p) if p.ends_with("/elm.json") => {
                fs::read(packages::dir().join(p.strip_prefix("/packages/")?)).ok()
            }
            (Method::Get, p) => website
                .get(&format!("https://package.elm-lang.org{p}"))
                .ok(),
            _ => None,
        },
    )
}

/// The transcript `release` recorded, or where it crashed or hung, the
/// one every other elm recorded alike.
fn transcript(expected: &str, release: Compat) -> Option<&str> {
    let own = recorded(expected, release)?;
    if !crashed(own) {
        return Some(own);
    }
    let mut others = (Compat::ALL.iter())
        .filter_map(|&release| recorded(expected, release))
        .filter(|transcript| !crashed(transcript));
    let other = others.next()?;
    others
        .all(|transcript| transcript == other)
        .then_some(other)
}
