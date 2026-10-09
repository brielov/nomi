//! nomi invoked as `elm`, against elm's own transcripts of `elm init` and
//! `elm install`: each case under `elm/cases/` must reproduce the
//! transcript of the same name under `expected/`, which `cargo xtask
//! oracle transcripts` records.
//!
//! Like the recordings, cases run offline, with a cache that knows and
//! holds the releases under `elm/packages/` and nothing else.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use nomi_base::Compat;
use nomi_project::Name;
use nomi_project::json::Cursor as JsonCursor;
use nomi_registry::{Http, HttpError, Index, Store};
use nomi_testkit::transcript::Case;
use sha1::{Digest, Sha1};

/// Cases nomi's transcript can't match here, and where they are tested
/// instead.
const ELSEWHERE: &[(&str, &str)] = &[(
    "install_no_args",
    "names the cache, whose path changes how the hint wraps; \
     crates/engine/src/install.rs tests it with a fixed one",
)];

#[test]
fn elm_transcripts() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/elm");
    let bin = tempfile::tempdir().unwrap();
    let elm = bin
        .path()
        .join(format!("elm{}", std::env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_nomi"), &elm).unwrap();
    let website = Website::new(&dir.join("packages"));
    let mut cases = Vec::new();
    for entry in fs::read_dir(dir.join("cases")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        if path.extension().is_some_and(|e| e == "case")
            && !ELSEWHERE.iter().any(|(case, _)| *case == name)
        {
            cases.push((name, path));
        }
    }
    cases.sort();
    assert!(cases.len() >= 20, "found only {} cases", cases.len());
    let mut failures = Vec::new();
    for (name, path) in &cases {
        let case = Case::parse(&fs::read_to_string(path).unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let expected = fs::read_to_string(dir.join("expected").join(format!("{name}.txt")))
            .unwrap_or_else(|e| panic!("{name}: {e}; rerun `cargo xtask oracle transcripts`"));
        let cache = tempfile::tempdir().unwrap();
        website.fill(cache.path());
        let work = tempfile::tempdir().unwrap();
        case.create_files(work.path()).unwrap();
        let actual = case
            .run(offline(&elm, cache.path()), work.path())
            .unwrap()
            .render();
        if actual != expected {
            failures.push(format!(
                "{name}:\n--- expected\n{expected}--- actual\n{actual}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} transcripts differ:\n\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

/// `nomi rm`, which upstream has no command for, takes out what `nomi
/// add` put in.
#[test]
fn rm_undoes_add() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/elm");
    let cache = tempfile::tempdir().unwrap();
    Website::new(&dir.join("packages")).fill(cache.path());
    let case =
        Case::parse(&fs::read_to_string(dir.join("cases/install_add.case")).unwrap()).unwrap();
    let work = tempfile::tempdir().unwrap();
    case.create_files(work.path()).unwrap();
    let elm_json = work.path().join("elm.json");
    let before = fs::read_to_string(&elm_json).unwrap();
    let nomi = |args: &[&str]| nomi(cache.path(), work.path(), args);
    nomi(&["add", "elm/http"]);
    assert_ne!(fs::read_to_string(&elm_json).unwrap(), before);
    let removed = nomi(&["rm", "elm/http"]);
    assert_eq!(fs::read_to_string(&elm_json).unwrap(), before, "{removed}");
    for gone in ["elm/http", "elm/bytes", "elm/file"] {
        assert!(removed.contains(gone), "{removed}");
    }
}

/// `nomi tree`, `outdated` and `update`, which upstream has no commands
/// for, on an application whose dependencies are the fixture registry's
/// only releases.
#[test]
fn tree_outdated_and_update() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/elm");
    let cache = tempfile::tempdir().unwrap();
    Website::new(&dir.join("packages")).fill(cache.path());
    let case =
        Case::parse(&fs::read_to_string(dir.join("cases/install_add.case")).unwrap()).unwrap();
    let work = tempfile::tempdir().unwrap();
    case.create_files(work.path()).unwrap();
    let nomi = |args: &[&str]| nomi(cache.path(), work.path(), args);
    assert_eq!(
        nomi(&["tree"]),
        "\
elm/browser 1.0.2
├── elm/core 1.0.5
├── elm/html 1.0.1
│   ├── elm/core 1.0.5
│   ├── elm/json 1.1.4
│   │   └── elm/core 1.0.5
│   └── elm/virtual-dom 1.0.5
│       ├── elm/core 1.0.5
│       └── elm/json 1.1.4 (*)
├── elm/json 1.1.4 (*)
├── elm/time 1.0.0
│   └── elm/core 1.0.5
├── elm/url 1.0.0
│   └── elm/core 1.0.5
└── elm/virtual-dom 1.0.5 (*)
elm/core 1.0.5
elm/html 1.0.1 (*)
"
    );
    assert_eq!(nomi(&["outdated"]), "Everything is up to date!\n");
    assert_eq!(nomi(&["update"]), "Everything is up to date!\n");
    nomi(&["add", "elm/http"]);
    assert_eq!(nomi(&["update", "elm/http"]), "Everything is up to date!\n");
}

/// `nomi check` on an application whose dependencies are the fixture
/// registry's: elm's reports for what nomi checks, its summary on
/// standard output, and the dependencies' modules known by name.
#[test]
fn check_reports_as_elm_make() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/elm");
    let cache = tempfile::tempdir().unwrap();
    Website::new(&dir.join("packages")).fill(cache.path());
    let case =
        Case::parse(&fs::read_to_string(dir.join("cases/install_add.case")).unwrap()).unwrap();
    let work = tempfile::tempdir().unwrap();
    case.create_files(work.path()).unwrap();
    let check = |args: &[&str]| {
        let output = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), cache.path())
            .arg("check")
            .args(args)
            .current_dir(work.path())
            .output()
            .unwrap();
        (
            output.status.success(),
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    };
    // The registry's packages hold only their elm.json, so a module that
    // checks cleanly uses none of their values.
    fs::write(
        work.path().join("src/Main.elm"),
        "module Main exposing (main)\n\nimport Html\n\n\nmain =\n    1\n",
    )
    .unwrap();
    let (ok, stdout, stderr) = check(&["src/Main.elm"]);
    assert!(ok, "{stdout}{stderr}");
    assert!(stdout.starts_with("No problems in syntax"), "{stdout}");

    fs::write(
        work.path().join("src/Main.elm"),
        "module Main exposing (main)\n\nimport Html\nimport Htm.Attributes\n\n\nmain =\n    Html.text \"hi\"\n",
    )
    .unwrap();
    let (ok, stdout, stderr) = check(&["src/Main.elm"]);
    assert!(!ok);
    assert_eq!(stdout, "Detected problems in 1 module.\n");
    let bar = stderr.lines().next().unwrap();
    assert!(bar.starts_with("-- MODULE NOT FOUND ---"), "{stderr}");
    assert!(
        bar.ends_with("--- src/Main.elm") && bar.chars().count() == 80,
        "{bar}"
    );
    assert!(stderr.contains("Html.Attributes"), "{stderr}");
    let (ok, stdout, stderr) = check(&["src/Main.elm", "--report", "json"]);
    assert!(!ok);
    assert_eq!(stdout, "");
    assert!(
        stderr.starts_with("{\"type\":\"compile-errors\",\"errors\":[{\"path\":"),
        "{stderr}"
    );

    let (ok, _, stderr) = check(&[]);
    assert!(!ok);
    assert!(stderr.starts_with("-- NO INPUT"), "{stderr}");
    assert!(stderr.contains("nomi check src/Main.elm"), "{stderr}");
}

/// The output of `nomi` with `args` in `work`, which must succeed,
/// answering yes to any question.
fn nomi(cache: &Path, work: &Path, args: &[&str]) -> String {
    let mut command = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), cache);
    let mut child = command
        .args(args)
        .current_dir(work)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"y\n").unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "nomi {args:?}:\n{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

/// `program` with the cache at `cache`, where every request fails, as
/// the recordings' did.
fn offline(program: &Path, cache: &Path) -> Command {
    let mut command = Command::new(program);
    command.env("NOMI_CACHE", cache);
    for var in [
        "NO_PROXY",
        "no_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
    ] {
        command.env_remove(var);
    }
    command.env("ALL_PROXY", "http://127.0.0.1:9");
    command
}

/// The registry the recordings had: one release of each fixture package,
/// whose archive holds its `elm.json`.
struct Website {
    archives: BTreeMap<(Name, String), Vec<u8>>,
}

impl Website {
    /// The releases under `packages`, at `<author>/<project>/<version>/`.
    fn new(packages: &Path) -> Website {
        let compat = Compat::LATEST;
        let mut archives = BTreeMap::new();
        for author in fs::read_dir(packages).unwrap() {
            let author = author.unwrap().path();
            for project in fs::read_dir(&author).unwrap() {
                let project = project.unwrap().path();
                for version in fs::read_dir(&project).unwrap() {
                    let version = version.unwrap().path();
                    let full = format!(
                        "{}/{}",
                        author.file_name().unwrap().to_string_lossy(),
                        project.file_name().unwrap().to_string_lossy()
                    );
                    let name =
                        Name::parse(full.as_bytes(), JsonCursor::start(compat), compat).unwrap();
                    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
                    let options = zip::write::SimpleFileOptions::default();
                    zip.add_directory("root/", options).unwrap();
                    zip.start_file("root/elm.json", options).unwrap();
                    zip.write_all(&fs::read(version.join("elm.json")).unwrap())
                        .unwrap();
                    let number = version.file_name().unwrap().to_string_lossy().into_owned();
                    archives.insert((name, number), zip.finish().unwrap().into_inner());
                }
            }
        }
        Website { archives }
    }

    /// Saves the index in the cache at `dir` and downloads every release
    /// into its store.
    fn fill(&self, dir: &Path) {
        let compat = Compat::LATEST;
        let index = Index::fetch(self, compat).unwrap();
        index.write(&dir.join("registry").join("index")).unwrap();
        let store = Store::new(dir.join("packages"));
        for name in self.archives.keys().map(|(name, _)| name) {
            let known = index.versions(name).unwrap();
            for version in known.all() {
                store.fetch(self, name, version, compat).unwrap();
            }
        }
    }

    fn archive(&self, url: &str) -> Option<&Vec<u8>> {
        self.archives.iter().find_map(|((name, version), archive)| {
            url.contains(&format!("/{name}/{version}/"))
                .then_some(archive)
        })
    }
}

impl Http for Website {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        let archive = self.archive(url).ok_or(HttpError::Status(404))?;
        if url.ends_with("endpoint.json") {
            let hash: String = Sha1::digest(archive)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            let zip = url.replace("endpoint.json", "archive.zip");
            return Ok(format!(r#"{{"url":"{zip}","hash":"{hash}"}}"#).into_bytes());
        }
        Ok(archive.clone())
    }

    fn post(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        assert!(url.ends_with("/all-packages"), "an unexpected POST {url}");
        let mut all = BTreeMap::<String, Vec<String>>::new();
        for (name, version) in self.archives.keys() {
            all.entry(name.to_string())
                .or_default()
                .push(format!("\"{version}\""));
        }
        let entries: Vec<String> = all
            .iter()
            .map(|(name, versions)| format!("\"{name}\":[{}]", versions.join(",")))
            .collect();
        Ok(format!("{{{}}}", entries.join(",")).into_bytes())
    }
}
