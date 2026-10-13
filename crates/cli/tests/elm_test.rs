//! nomi invoked as `elm-test`, against elm-test 0.19.3-0's own
//! transcripts: each case under `elm_test/cases/` must reproduce the
//! transcript of the same name under `expected/`, which `cargo xtask
//! oracle transcripts elm-test` records, normalized as the recordings
//! are.
//!
//! Like the recordings, cases run offline, with a cache that holds the
//! fixture packages and nothing else. The tests run in Node.

#[path = "support/website.rs"]
mod website;

use std::fs;
use std::path::Path;
use std::process::Command;

use nomi_testkit::transcript::{Case, normalize_elm_test};
use website::{Website, offline};

#[test]
fn elm_test_transcripts() {
    if Command::new("node").arg("--version").output().is_err() && std::env::var_os("CI").is_none() {
        eprintln!("skipped: elm-test's runs need Node");
        return;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/elm_test");
    let bin = tempfile::tempdir().unwrap();
    let elm_test = bin
        .path()
        .join(format!("elm-test{}", std::env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_nomi"), &elm_test).unwrap();
    let packages = Path::new(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/packages");
    let cache = tempfile::tempdir().unwrap();
    Website::with_sources(&packages).fill(cache.path());
    let mut cases = Vec::new();
    for entry in fs::read_dir(dir.join("cases")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        if path.extension().is_some_and(|e| e == "case") {
            cases.push((name, path));
        }
    }
    cases.sort();
    assert!(cases.len() >= 30, "found only {} cases", cases.len());
    let mut failures = Vec::new();
    for (name, path) in &cases {
        let case = Case::parse(&fs::read_to_string(path).unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let expected = fs::read_to_string(dir.join("expected").join(format!("{name}.txt")))
            .unwrap_or_else(|e| panic!("{name}: {e}; rerun `cargo xtask oracle transcripts`"));
        let work = tempfile::tempdir().unwrap();
        let work = nomi_engine::canonical(work.path()).unwrap();
        let root = work.join("project");
        case.create_files(&root).unwrap();
        let mut command = offline(&elm_test, cache.path());
        command.env_remove("FORCE_COLOR").env_remove("NO_COLOR");
        let mut transcript = case.run(command, &root).unwrap();
        transcript
            .files
            .retain(|(path, _)| !path.starts_with("elm-stuff/"));
        let dir = root.display().to_string();
        let home = cache.path().display().to_string();
        transcript.stdout = normalize_elm_test(&transcript.stdout, &dir, &home);
        transcript.stderr = normalize_elm_test(&transcript.stderr, &dir, &home);
        let actual = transcript.render();
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

/// `elm-test --watch` runs the tests, then again when a test file
/// changes, naming it, and keeps watching after a run that fails to
/// build.
#[test]
fn watch_runs_again_on_changes() {
    use std::io::Read;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    if Command::new("node").arg("--version").output().is_err() && std::env::var_os("CI").is_none() {
        eprintln!("skipped: elm-test's runs need Node");
        return;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/elm_test");
    let packages = Path::new(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/packages");
    let cache = tempfile::tempdir().unwrap();
    Website::with_sources(&packages).fill(cache.path());
    let case =
        Case::parse(&fs::read_to_string(dir.join("cases/passing_console.case")).unwrap()).unwrap();
    let work = tempfile::tempdir().unwrap();
    let root = nomi_engine::canonical(work.path()).unwrap();
    case.create_files(&root).unwrap();
    let mut command = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), cache.path());
    let mut child = command
        .args([
            "test",
            "--watch",
            "--no-color",
            "--workers",
            "1",
            "tests/Passing",
        ])
        .current_dir(&root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let output = Arc::new(Mutex::new(String::new()));
    for stream in [
        Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
        Box::new(child.stderr.take().unwrap()),
    ] {
        let output = Arc::clone(&output);
        std::thread::spawn(move || {
            let mut stream = stream;
            let mut buffer = [0; 4096];
            while let Ok(n) = stream.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                let text = String::from_utf8_lossy(&buffer[..n]).into_owned();
                output.lock().unwrap().push_str(&text);
            }
        });
    }
    let wait_for = |what: &str, times: usize| {
        let started = Instant::now();
        loop {
            let seen = output.lock().unwrap().matches(what).count();
            if seen >= times {
                return;
            }
            assert!(
                started.elapsed() <= Duration::from_secs(120),
                "no {times} of {what:?} in:\n{}",
                output.lock().unwrap()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    let watching = "Watching for changes...";
    wait_for("TEST RUN PASSED", 1);
    wait_for(watching, 1);
    let one = root.join("tests/Passing/One.elm");
    let text = fs::read_to_string(&one).unwrap();
    fs::write(&one, format!("{text}\n-- changed\n")).unwrap();
    wait_for(
        &format!("{} changed", Path::new("tests/Passing/One.elm").display()),
        1,
    );
    wait_for("TEST RUN PASSED", 2);
    wait_for(watching, 2);
    fs::write(&one, format!("{text}\n{{-\n")).unwrap();
    wait_for("`elm make` failed with exit code 1.", 1);
    wait_for(watching, 3);
    let _ = child.kill();
    let _ = child.wait();
    let output = output.lock().unwrap();
    // The console cleared before each run.
    assert_eq!(output.matches("\u{1b}[2J").count(), 3, "{output}");
}
