//! Discovery and the runner over node-test-runner's fixture suites:
//! the files and modules elm-test's JSON transcripts ran, its errors,
//! and the runner it would build.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nomi_engine::console::Captured;
use nomi_engine::paths::Cache;
use nomi_engine::test_deps::Solve;
use nomi_engine::{Context, Http, HttpError, Vfs};
use nomi_project::Outline;
use nomi_test::{
    Ended, Env, Failure, Options, Parsed, Project, Report, Run, Runner, Supervise, build, modules,
    parse, resolve,
};
use nomi_testkit::packages;
use nomi_testkit::transcript::{Case, Transcript, normalize_elm_test};

/// node-test-runner's suites, each a case for every report.
const SUITES: [&str; 6] = [
    "passing",
    "failing",
    "distribution",
    "nested",
    "runtime_exception",
    "invalid_xml",
];

/// The workers where a case names none: several, which the supervisor
/// puts in order.
const WORKERS: usize = 3;

fn cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../cli/tests/elm_test")
}

/// A cache whose store holds the fixture packages, laid out as nomi keeps
/// downloads.
fn cache(dir: &Path) -> Cache {
    let cache = Cache::new(dir.to_owned());
    packages::store(&cache.packages()).unwrap();
    cache
}

/// What elm-test printed for the case.
fn transcript(name: &str) -> Transcript {
    let text =
        fs::read_to_string(cases_dir().join("expected").join(format!("{name}.txt"))).unwrap();
    Transcript::parse(&text).unwrap()
}

fn text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).unwrap()
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
            let mut index = serde_json::Map::new();
            for package in packages::manifest() {
                let versions = index.entry(package.name).or_insert(serde_json::json!([]));
                versions
                    .as_array_mut()
                    .unwrap()
                    .push(package.version.into());
            }
            return Ok(serde_json::to_vec(&index).unwrap());
        }
        if url.contains("/all-packages/since/") {
            return Ok(b"[]".to_vec());
        }
        Err(HttpError::Transport(format!("offline: {url}")))
    }
}

/// A case set up in a directory of its own: its project, and the run its
/// arguments ask for.
struct Prepared {
    dir: PathBuf,
    project: Project,
    run: Run,
}

impl Prepared {
    /// The case `name`, in a directory named so under `temp`.
    fn new(temp: &Path, name: &str) -> Prepared {
        let path = cases_dir().join("cases").join(format!("{name}.case"));
        let case = Case::parse(&fs::read_to_string(path).unwrap()).unwrap();
        let dir = nomi_engine::canonical(temp).unwrap().join(name);
        case.create_files(&dir).unwrap();
        let Parsed::Run(flags, globs) = parse(&case.args, WORKERS) else {
            panic!("{name} is no run");
        };
        Prepared {
            project: Project::read(&dir).unwrap(),
            dir,
            run: Run {
                flags,
                globs,
                color: case.args.iter().any(|a| a == "--color"),
                cores: WORKERS,
            },
        }
    }

    /// The files the run's globs find.
    fn files(&self) -> Result<Vec<PathBuf>, String> {
        resolve(&self.project, &self.dir, &self.run.globs)
    }

    fn options(&self, files: &[PathBuf]) -> Options {
        self.run
            .options(files, self.run.flags.seed.expect("a seed"))
    }

    /// The runner of `files` with `options`, built offline with the
    /// fixture packages, and what it showed.
    fn build(
        &self,
        cache: &Cache,
        files: &[PathBuf],
        options: &Options,
    ) -> (Result<Runner, Failure>, Captured) {
        let cx = Context {
            vfs: &Vfs::new(),
            dir: &self.dir,
            cache,
            http: &Offline,
        };
        let env = Env {
            cx: &cx,
            solve: Solve::default(),
            color: false,
        };
        let mut console = Captured::default();
        let runner = build(
            &env,
            &mut console,
            &self.project,
            files,
            options,
            &mut |_, _| {},
        );
        (runner, console)
    }
}

#[test]
fn discovery_finds_what_elm_test_ran() {
    nomi_testkit::require_node();
    let temp = tempfile::tempdir().unwrap();
    let cache = cache(&temp.path().join("cache"));
    for suite in SUITES {
        let name = format!("{suite}_json");
        let case = Prepared::new(temp.path(), &name);
        let files = case.files().unwrap();
        // The tests, by the types of the runner's build.
        let (runner, console) = case.build(&cache, &files, &case.options(&files));
        let runner = match runner {
            Ok(runner) => runner,
            Err(failure) => panic!("{name}: {failure:?}\n{}", console.err),
        };
        assert_eq!(
            (console.out.as_str(), console.err.as_str()),
            ("", ""),
            "{name}"
        );
        let expected = transcript(&name);
        let (stdout, stderr) = (text(&expected.stdout), text(&expected.stderr));
        let loaded = load_in_node(&runner);
        // A runtime exception ends the run before it starts reporting.
        let Some(start) = stdout
            .lines()
            .find_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        else {
            assert!(!runner.tests.is_empty(), "{name}");
            let error = loaded.unwrap_err();
            assert!(error.contains(stderr.trim()), "{name}: {error}");
            continue;
        };
        loaded.unwrap_or_else(|e| panic!("{name}: {e}"));
        let ran: Vec<String> = (start["paths"].as_array().unwrap().iter())
            .map(|p| {
                p.as_str()
                    .unwrap()
                    .replace("{dir}", &case.dir.display().to_string())
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
/// when it doesn't.
fn load_in_node(runner: &Runner) -> Result<(), String> {
    let script = "const run = require(process.argv[1]).run; \
        const send = run(0, () => {}); \
        if (typeof send !== 'function') process.exit(1);";
    let output = Command::new("node")
        .args(["-e", script])
        .arg(&runner.js)
        .output()
        .unwrap_or_else(|e| panic!("node: {e}"));
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

#[test]
fn compile_errors_are_elms() {
    let temp = tempfile::tempdir().unwrap();
    let cache = cache(&temp.path().join("cache"));
    let case = Prepared::new(temp.path(), "compile_error");
    let files = case.files().unwrap();
    let (runner, console) = case.build(&cache, &files, &case.options(&files));
    assert!(matches!(runner, Err(Failure::Shown)), "{runner:?}");
    let stderr = transcript("compile_error").stderr;
    let expected = text(&stderr).replace("{dir}", &case.dir.display().to_string());
    // elm-test says how `elm make` ended after its reports.
    let expected = (expected.trim_end())
        .strip_suffix("`elm make` failed with exit code 1.")
        .unwrap();
    assert_eq!(console.err, expected);
}

#[test]
fn errors_are_elm_tests() {
    let temp = tempfile::tempdir().unwrap();
    let case = Prepared::new(temp.path(), "no_files");
    let stderr = transcript("no_files").stderr;
    assert_eq!(case.files().unwrap_err(), text(&stderr).trim_end());
    // A file in no source directory, and one whose path makes no module.
    let other = case.dir.join("other/A.elm");
    fs::create_dir_all(other.parent().unwrap()).unwrap();
    fs::write(&other, "module A exposing (..)\n").unwrap();
    let error = modules(&case.project, &[other]).unwrap_err();
    assert!(error.contains("…matches no source directory!"), "{error}");
    let lower = case.dir.join("src/lower/A.elm");
    fs::create_dir_all(lower.parent().unwrap()).unwrap();
    fs::write(&lower, "module A exposing (..)\n").unwrap();
    let error = modules(&case.project, &[lower]).unwrap_err();
    assert!(error.contains("lower.A"), "{error}");
}

/// `text` as elm-test's transcripts normalize it, `dir` as `{dir}` and
/// `home` as `{home}`.
fn normalize(text: &str, dir: &Path, home: &Path) -> String {
    let (dir, home) = (dir.display().to_string(), home.display().to_string());
    String::from_utf8(normalize_elm_test(text.as_bytes(), &dir, &home)).unwrap()
}

#[test]
fn runs_print_what_elm_test_printed() {
    nomi_testkit::require_node();
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("cache");
    let cache = cache(&home);
    let mut names: Vec<String> = (SUITES.iter())
        .flat_map(|s| ["json", "console", "color", "junit"].map(|r| format!("{s}_{r}")))
        .collect();
    names.extend(["no_tests".to_owned(), "no_tests_json".to_owned()]);
    for name in names {
        let case = Prepared::new(temp.path(), &name);
        let files = case.files().unwrap();
        let options = case.options(&files);
        let (built, mut console) = case.build(&cache, &files, &options);
        let runner = built.unwrap_or_else(|e| panic!("{name}: {e:?}\n{}", console.err));
        let supervise = Supervise {
            report: options.report,
            processes: options.processes,
            color: case.run.color,
        };
        // One worker's exception is the command's to report.
        let code = match supervise.run(&mut console, &runner.js).unwrap() {
            Ended::Code(code) => code,
            Ended::Thrown => 1,
        };
        let expected = transcript(&name);
        let (stdout, stderr) = (text(&expected.stdout), text(&expected.stderr));
        assert_eq!(
            Some(code),
            expected.exit,
            "{name}'s exit code\n{}",
            console.err
        );
        let stderr_lines = |s: &str| s.lines().map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(
            stderr_lines(&normalize(&console.err, &case.dir, &home)),
            stderr_lines(stderr),
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
            _ => stdout,
        };
        assert_eq!(
            normalize(&console.out, &case.dir, &home).trim_end_matches('\n'),
            expected.trim_end_matches('\n'),
            "{name}'s standard output"
        );
    }
}
