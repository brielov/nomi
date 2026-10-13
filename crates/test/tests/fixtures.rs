//! Discovery and the runner over node-test-runner's fixture suites:
//! the files and modules elm-test's JSON
//! transcripts ran, its errors, and the runner it would build.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use nomi_engine::console::Console;
use nomi_engine::paths::Cache;
use nomi_engine::test_deps::Solve;
use nomi_engine::{Http, HttpError, Vfs};
use nomi_project::Outline;
use nomi_test::supervise::{self, Ended, Supervise};
use nomi_test::{Env, Failure, Options, Project, Report, Runner, build, modules, resolve};
use nomi_testkit::transcript::Case;

fn cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../cli/tests/elm_test")
}

fn packages_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/packages")
}

/// A cache whose store holds the fixture packages, laid out as nomi keeps
/// downloads.
fn cache(dir: &Path) -> Cache {
    let cache = Cache::new(dir.to_owned());
    store(&cache.packages());
    cache
}

fn store(dir: &Path) {
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
        let end = if rest.starts_with("=== ") {
            0
        } else {
            rest.find("\n=== ").map_or(rest.len(), |i| i + 1)
        };
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

/// The website, listing the fixture packages alone, which are all
/// downloaded.
struct Offline;

impl Http for Offline {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        Err(HttpError::Transport(format!("offline: {url}")))
    }

    fn post(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        if url.ends_with("/all-packages") {
            let manifest = fs::read_to_string(packages_dir().join("manifest.toml")).unwrap();
            let mut index = serde_json::Map::new();
            let mut name = String::new();
            for line in manifest.lines() {
                let value = || line.split('"').nth(1).unwrap_or_default().to_owned();
                if line.starts_with("name = ") {
                    name = value();
                } else if line.starts_with("version = ") {
                    let versions = index.entry(name.clone()).or_insert(serde_json::json!([]));
                    versions.as_array_mut().unwrap().push(value().into());
                }
            }
            return Ok(serde_json::to_vec(&index).unwrap());
        }
        if url.contains("/all-packages/since/") {
            return Ok(b"[]".to_vec());
        }
        Err(HttpError::Transport(format!("offline: {url}")))
    }
}

/// A console keeping what it's given.
#[derive(Default)]
struct Kept {
    out: String,
    err: String,
}

impl Console for Kept {
    fn out(&mut self, text: &str) {
        self.out.push_str(text);
    }

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

fn options(files: &[PathBuf]) -> Options {
    Options {
        runs: 100,
        report: Report::Json,
        seed: 12345,
        processes: 1,
        globs: Vec::new(),
        paths: files.iter().map(|f| f.display().to_string()).collect(),
    }
}

/// A build's environment, offline with the fixture packages.
fn env<'a>(vfs: &'a Vfs, cache: &'a Cache) -> Env<'a, Offline> {
    Env {
        vfs,
        cache,
        http: &Offline,
        solve: Solve::default(),
        color: false,
    }
}

/// The runner of the case in `dir`, and what it showed.
fn runner(cache: &Cache, dir: &Path, case: &Case) -> (Result<Runner, Failure>, Kept) {
    let project = Project::read(dir).unwrap();
    let files = resolve(&project, dir, &globs(case)).unwrap();
    let mut console = Kept::default();
    let vfs = Vfs::new();
    let runner = build(
        &env(&vfs, cache),
        &mut console,
        &project,
        &files,
        &options(&files),
        &mut |_, _| {},
    );
    (runner, console)
}

#[test]
fn discovery_finds_what_elm_test_ran() {
    let temp = tempfile::tempdir().unwrap();
    let cache = cache(&temp.path().join("cache"));
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
        // The tests, by the types of the runner's build.
        let (runner, console) = runner(&cache, &dir, &case);
        let runner = match runner {
            Ok(runner) => runner,
            Err(failure) => panic!("{name}: {failure:?}\n{}", console.err),
        };
        assert_eq!(
            (console.out.as_str(), console.err.as_str()),
            ("", ""),
            "{name}"
        );
        let (stdout, stderr) = transcript(&name);
        let loaded = load_in_node(&runner);
        // A runtime exception ends the run before it starts reporting.
        let Some(start) = stdout
            .lines()
            .find_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        else {
            assert!(!runner.tests.is_empty(), "{name}");
            if let Some(loaded) = loaded {
                let error = loaded.unwrap_err();
                assert!(error.contains(stderr.trim()), "{name}: {error}");
            }
            continue;
        };
        if let Some(loaded) = loaded {
            loaded.unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        let ran: Vec<String> = (start["paths"].as_array().unwrap().iter())
            .map(|p| {
                p.as_str()
                    .unwrap()
                    .replace("{dir}", &dir.display().to_string())
            })
            .collect();
        let project = Project::read(&dir).unwrap();
        let files = resolve(&project, &dir, &globs(&case)).unwrap();
        let found: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
        assert_eq!(found, ran, "{name}'s files");
        // Each module with a test, as the reports name them.
        let reported: BTreeSet<String> = (stdout.lines())
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|e| e["event"] == "testCompleted")
            .filter_map(|e| e["labels"][0].as_str().map(str::to_owned))
            .collect();
        let with_tests: BTreeSet<String> = runner.tests.iter().map(|(m, _)| m.clone()).collect();
        assert_eq!(with_tests, reported, "{name}'s modules with tests");
        if suite == "passing" {
            let unexposed = runner.tests.iter().find(|(m, _)| m == "Passing.Unexposed");
            assert_eq!(unexposed.unwrap().1, ["all"]);
            dependencies_are_elm_tests(&runner);
        }
    }
}

/// The dependencies elm-test 0.19.3-0 solves for the passing suite: its
/// direct and test-direct ones and the runner's, direct; the rest the
/// newest downloaded.
fn dependencies_are_elm_tests(runner: &Runner) {
    let bytes = fs::read(runner.dir.join("elm.json")).unwrap();
    let Ok(Outline::App(app)) = Outline::decode(&bytes, nomi_base::Compat::V0_19_3) else {
        panic!("the generated elm.json is no application");
    };
    let listed = |deps: &std::collections::BTreeMap<_, _>| -> Vec<String> {
        deps.iter().map(|(n, v)| format!("{n} {v}")).collect()
    };
    assert_eq!(
        listed(&app.direct),
        [
            "elm/core 1.0.5",
            "elm/json 1.1.4",
            "elm/random 1.0.0",
            "elm/time 1.0.0",
            "elm-explorations/test 2.2.1",
        ]
    );
    assert_eq!(
        listed(&app.indirect),
        ["elm/bytes 1.0.8", "elm/html 1.0.1", "elm/virtual-dom 1.0.5"]
    );
    assert!(app.test_direct.is_empty() && app.test_indirect.is_empty());
}

/// Whether Node loads the runner and starts its `main`, or what it says
/// when it doesn't; `None` without Node, except in CI, which has it.
fn load_in_node(runner: &Runner) -> Option<Result<(), String>> {
    let script = "const run = require(process.argv[1]).run; \
        const send = run(0, () => {}); \
        if (typeof send !== 'function') process.exit(1);";
    let output = Command::new("node")
        .args(["-e", script])
        .arg(&runner.js)
        .output();
    let output = match output {
        Ok(output) => output,
        Err(e) if e.kind() == io::ErrorKind::NotFound && std::env::var_os("CI").is_none() => {
            return None;
        }
        Err(e) => panic!("node: {e}"),
    };
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Some(if output.status.success() {
        Ok(())
    } else {
        Err(stderr)
    })
}

#[test]
fn compile_errors_are_elms() {
    let temp = tempfile::tempdir().unwrap();
    let cache = cache(&temp.path().join("cache"));
    let case = case("compile_error");
    let dir = nomi_engine::canonical(temp.path()).unwrap().join("project");
    case.create_files(&dir).unwrap();
    let (runner, console) = runner(&cache, &dir, &case);
    assert!(matches!(runner, Err(Failure::Shown)), "{runner:?}");
    let (_, stderr) = transcript("compile_error");
    let expected = stderr.replace("{dir}", &dir.display().to_string());
    // elm-test says how `elm make` ended after its reports.
    let expected = (expected.trim_end())
        .strip_suffix("`elm make` failed with exit code 1.")
        .unwrap();
    assert_eq!(console.err, expected);
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

/// The options a case's arguments give, with `workers` workers where it
/// names none; and whether elm-test colors its own messages.
fn case_options(case: &Case, files: &[PathBuf], workers: usize) -> (Options, bool) {
    let value = |name: &str| {
        let at = case.args.iter().position(|a| a == name)?;
        case.args.get(at + 1).cloned()
    };
    let color = case.args.iter().any(|a| a == "--color");
    let report = match value("--report").as_deref() {
        Some("json") => Report::Json,
        Some("junit") => Report::JUnit,
        _ => Report::Console { color },
    };
    let options = Options {
        runs: value("--fuzz").map_or(100, |v| v.parse().unwrap()),
        report,
        seed: value("--seed").unwrap().parse().unwrap(),
        processes: value("--workers").map_or(workers, |v| v.parse().unwrap()),
        globs: globs(case),
        paths: files.iter().map(|f| f.display().to_string()).collect(),
    };
    (options, color)
}

/// `text` with the durations elm-test's transcripts normalize, and `dir`
/// as `{dir}`.
fn normalize(text: &str, dir: &Path) -> String {
    let mut text = text.replace(&dir.display().to_string(), "{dir}");
    for marker in [
        "\"duration\":\"",
        " time=\"",
        "Duration: ",
        "Duration: \u{1b}[22m",
    ] {
        let mut out = String::new();
        let mut rest = text.as_str();
        while let Some(at) = rest.find(marker) {
            out.push_str(&rest[..at + marker.len()]);
            rest = &rest[at + marker.len()..];
            let end =
                (rest.find(|c: char| !(c.is_ascii_digit() || c == '.'))).unwrap_or(rest.len());
            if end > 0 {
                out.push('0');
            }
            rest = &rest[end..];
        }
        out.push_str(rest);
        text = out;
    }
    text
}

#[test]
fn runs_print_what_elm_test_printed() {
    if Command::new("node").arg("--version").output().is_err() && std::env::var_os("CI").is_none() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let cache = cache(&temp.path().join("cache"));
    let suites = [
        "passing",
        "failing",
        "distribution",
        "nested",
        "runtime_exception",
        "invalid_xml",
    ];
    let mut names: Vec<String> = (suites.iter())
        .flat_map(|s| ["json", "console", "color", "junit"].map(|r| format!("{s}_{r}")))
        .collect();
    names.extend(["no_tests".to_owned(), "no_tests_json".to_owned()]);
    for name in names {
        let case = case(&name);
        let dir = nomi_engine::canonical(temp.path()).unwrap().join(&name);
        case.create_files(&dir).unwrap();
        let project = Project::read(&dir).unwrap();
        let files = resolve(&project, &dir, &globs(&case)).unwrap();
        let (options, color) = case_options(&case, &files, 3);
        let mut console = Kept::default();
        let vfs = Vfs::new();
        let built = build(
            &env(&vfs, &cache),
            &mut console,
            &project,
            &files,
            &options,
            &mut |_, _| {},
        );
        let runner = built.unwrap_or_else(|e| panic!("{name}: {e:?}\n{}", console.err));
        let supervise = Supervise {
            report: options.report,
            processes: options.processes,
            color,
        };
        // One worker's exception is the command's to report.
        let code = match supervise::run(&mut console, &runner.js, supervise).unwrap() {
            Ended::Code(code) => code,
            Ended::Thrown => 1,
        };
        let text =
            fs::read_to_string(cases_dir().join("expected").join(format!("{name}.txt"))).unwrap();
        let expected_code: i32 = text
            .strip_prefix("=== exit ")
            .and_then(|t| t.lines().next())
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(code, expected_code, "{name}'s exit code\n{}", console.err);
        let (stdout, stderr) = transcript(&name);
        let stderr_lines = |s: &str| s.lines().map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(
            stderr_lines(&normalize(&console.err, &dir)),
            stderr_lines(&stderr),
            "{name}'s standard error"
        );
        if code == 1 {
            // elm-test's own lines, its progress and the line after it, are
            // the command's.
            continue;
        }
        // The console's progress line is the command's too.
        let expected = match options.report {
            Report::Console { .. } => stdout.split_once('\n').unwrap().1,
            _ => stdout.as_str(),
        };
        assert_eq!(
            normalize(&console.out, &dir).trim_end_matches('\n'),
            expected.trim_end_matches('\n'),
            "{name}'s standard output"
        );
    }
}
