//! Discovery over node-test-runner's fixture suites (plan 10.2): the files
//! and modules elm-test's JSON transcripts ran, and its errors.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nomi_engine::{Store, Vfs};
use nomi_test::{Project, modules, resolve, tests};
use nomi_testkit::transcript::Case;

fn cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../cli/tests/elm_test")
}

fn packages_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/packages")
}

/// A store holding the fixture packages, laid out as nomi's cache keeps
/// downloads.
fn store(dir: &Path) -> Store {
    let manifest = fs::read_to_string(packages_dir().join("manifest.toml")).unwrap();
    let (mut name, mut version) = (String::new(), String::new());
    for line in manifest.lines() {
        let value = || line.split('"').nth(1).unwrap_or_default().to_owned();
        if line.starts_with("name = ") {
            name = value();
        } else if line.starts_with("version = ") {
            version = value();
        } else if line.starts_with("sha1 = ") {
            let hash = value();
            let release = dir.join("releases").join(&name).join(&version);
            fs::create_dir_all(release.parent().unwrap()).unwrap();
            fs::write(&release, format!("{hash}\n")).unwrap();
            copy_tree(
                &packages_dir().join(&name).join(&version),
                &dir.join("sources").join(&hash),
            );
        }
    }
    Store::new(dir.to_owned())
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &to.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

/// The case's globs: its arguments that are neither options nor their
/// values.
fn globs(case: &Case) -> Vec<String> {
    let mut globs = Vec::new();
    let mut args = case.args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seed" | "--fuzz" | "--report" | "--workers" | "--compiler" => {
                args.next();
            }
            other if other.starts_with("--") => {}
            other => globs.push(other.to_owned()),
        }
    }
    globs
}

/// The case's transcript: its standard output and error, unquoted.
fn transcript(name: &str) -> (String, String) {
    let text =
        fs::read_to_string(cases_dir().join("expected").join(format!("{name}.txt"))).unwrap();
    let section = |header: &str| -> String {
        let Some(start) = text.find(header) else {
            return String::new();
        };
        let rest = &text[start + header.len()..];
        let end = rest.find("\n=== ").map_or(rest.len(), |i| i + 1);
        rest[..end]
            .lines()
            .map(|l| l.strip_prefix('|').unwrap_or(l))
            .collect::<Vec<_>>()
            .join("\n")
    };
    (section("=== stdout\n"), section("=== stderr\n"))
}

fn case(name: &str) -> Case {
    Case::parse(
        &fs::read_to_string(cases_dir().join("cases").join(format!("{name}.case"))).unwrap(),
    )
    .unwrap()
}

#[test]
fn discovery_finds_what_elm_test_ran() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp.path().join("store"));
    let suites = [
        "passing",
        "failing",
        "distribution",
        "nested",
        "runtime_exception",
        "invalid_xml",
    ];
    for suite in suites {
        let name = format!("{suite}_json");
        let case = case(&name);
        let dir = nomi_engine::canonical(temp.path()).unwrap().join(&name);
        case.create_files(&dir).unwrap();
        let project = Project::read(&dir).unwrap();
        let files = resolve(&project, &dir, &globs(&case)).unwrap();
        let (stdout, _) = transcript(&name);
        // A runtime exception ends the run before it starts reporting.
        let Some(start) = stdout
            .lines()
            .find_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        else {
            assert!(!modules(&project, &files).unwrap().is_empty(), "{name}");
            continue;
        };
        let ran: Vec<String> = (start["paths"].as_array().unwrap().iter())
            .map(|p| {
                p.as_str()
                    .unwrap()
                    .replace("{dir}", &dir.display().to_string())
            })
            .collect();
        let found: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
        assert_eq!(found, ran, "{name}'s files");
        // Each module with a test, as the reports name them.
        let reported: BTreeSet<String> = (stdout.lines())
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|e| e["event"] == "testCompleted")
            .filter_map(|e| e["labels"][0].as_str().map(str::to_owned))
            .collect();
        let found = modules(&project, &files).unwrap();
        // The types, from the project as elm builds it: its test
        // dependencies among its own, `tests/` among its sources, as the
        // runner's generated project will have them (plan 10.3).
        buildable(&dir);
        let vfs = Vfs::new();
        let index = nomi_engine::index::index(
            &vfs.snapshot(),
            &store,
            &found[0].path,
            &mut Default::default(),
        )
        .unwrap();
        let with_tests: BTreeSet<String> = (found.iter())
            .filter(|m| {
                index
                    .interfaces
                    .get(&m.name)
                    .is_some_and(|i| !tests(m, i).is_empty())
            })
            .map(|m| m.name.clone())
            .collect();
        assert_eq!(with_tests, reported, "{name}'s modules with tests");
        if suite == "passing" {
            let unexposed = found
                .iter()
                .find(|m| m.name == "Passing.Unexposed")
                .unwrap();
            assert_eq!(
                tests(unexposed, &index.interfaces["Passing.Unexposed"]),
                ["all"]
            );
        }
    }
}

/// The project in `dir` with every dependency a direct one and `tests/`
/// a source directory, which elm builds.
fn buildable(dir: &Path) {
    let path = dir.join("elm.json");
    let mut json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let mut all = serde_json::Map::new();
    for section in ["dependencies", "test-dependencies"] {
        for kind in ["direct", "indirect"] {
            if let Some(deps) = json[section][kind].as_object() {
                all.extend(deps.clone());
            }
        }
    }
    json["dependencies"] = serde_json::json!({ "direct": all, "indirect": {} });
    json["test-dependencies"] = serde_json::json!({ "direct": {}, "indirect": {} });
    json["source-directories"] = serde_json::json!(["src", "tests"]);
    fs::write(&path, serde_json::to_string_pretty(&json).unwrap()).unwrap();
}

#[test]
fn errors_are_elm_tests() {
    let temp = tempfile::tempdir().unwrap();
    let case = case("no_files");
    let dir = nomi_engine::canonical(temp.path()).unwrap();
    case.create_files(&dir).unwrap();
    let project = Project::read(&dir).unwrap();
    let error = resolve(&project, &dir, &globs(&case)).unwrap_err();
    let (_, stderr) = transcript("no_files");
    assert_eq!(error, stderr.trim_end());
    // A file in no source directory, and one whose path makes no module.
    fs::create_dir_all(dir.join("other")).unwrap();
    fs::write(dir.join("other/A.elm"), "module A exposing (..)\n").unwrap();
    let error = modules(&project, &[dir.join("other/A.elm")]).unwrap_err();
    assert!(error.contains("…matches no source directory!"), "{error}");
    fs::create_dir_all(dir.join("src/lower")).unwrap();
    fs::write(dir.join("src/lower/A.elm"), "module A exposing (..)\n").unwrap();
    let error = modules(&project, &[dir.join("src/lower/A.elm")]).unwrap_err();
    assert!(error.contains("lower.A"), "{error}");
}
