//! nomi invoked as `elm`, against elm's own transcripts of `elm init` and
//! `elm install`: each case under `elm/cases/` must reproduce the
//! transcript of the same name under `expected/`, which `cargo xtask
//! oracle transcripts` records.
//!
//! Like the recordings, cases run offline, with a cache that knows and
//! holds the releases under `elm/packages/` and nothing else.

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Stdio;

use nomi_testkit::transcript::{Case, check_transcripts};
use tempfile::TempDir;
use website::{Website, offline};

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
    let (_bin, elm) = website::personality("elm");
    let website = Website::new(&dir.join("packages"));
    let ran = check_transcripts(&dir.join("cases"), &dir.join("expected"), |name, case| {
        if ELSEWHERE.iter().any(|(case, _)| *case == name) {
            return None;
        }
        let cache = tempfile::tempdir().unwrap();
        website.fill(cache.path());
        let work = tempfile::tempdir().unwrap();
        case.create_files(work.path()).unwrap();
        let transcript = case.run(offline(&elm, cache.path()), work.path());
        Some(transcript.unwrap().render())
    });
    assert!(ran >= 20, "ran only {ran} cases");
}

/// The `install_add` case's application, with a cache that knows the
/// fixture registry's releases.
struct Project {
    cache: TempDir,
    work: TempDir,
}

impl Project {
    fn new() -> Project {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/elm");
        let cache = tempfile::tempdir().unwrap();
        Website::new(&dir.join("packages")).fill(cache.path());
        let case = fs::read_to_string(dir.join("cases/install_add.case")).unwrap();
        let work = tempfile::tempdir().unwrap();
        Case::parse(&case)
            .unwrap()
            .create_files(work.path())
            .unwrap();
        Project { cache, work }
    }

    /// The output of `nomi` with `args` here, which must succeed,
    /// answering yes to any question.
    fn nomi(&self, args: &[&str]) -> String {
        let mut command = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), self.cache.path());
        let mut child = command
            .args(args)
            .current_dir(self.work.path())
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
}

/// `nomi rm`, which upstream has no command for, takes out what `nomi
/// add` put in.
#[test]
fn rm_undoes_add() {
    let project = Project::new();
    let elm_json = project.work.path().join("elm.json");
    let before = fs::read_to_string(&elm_json).unwrap();
    project.nomi(&["add", "elm/http"]);
    assert_ne!(fs::read_to_string(&elm_json).unwrap(), before);
    let removed = project.nomi(&["rm", "elm/http"]);
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
    let project = Project::new();
    assert_eq!(
        project.nomi(&["tree"]),
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
    assert_eq!(project.nomi(&["outdated"]), "Everything is up to date!\n");
    assert_eq!(project.nomi(&["update"]), "Everything is up to date!\n");
    project.nomi(&["add", "elm/http"]);
    assert_eq!(
        project.nomi(&["update", "elm/http"]),
        "Everything is up to date!\n"
    );
}

/// `nomi check` on an application whose dependencies are the fixture
/// registry's: elm's reports for what nomi checks, its summary on
/// standard output, and the dependencies' modules known by name.
#[test]
fn check_reports_as_elm_make() {
    let Project { cache, work } = Project::new();
    let check = |args: &[&str]| {
        let output = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), cache.path())
            .arg("check")
            .args(args)
            .current_dir(work.path())
            .output()
            .unwrap();
        Ran {
            ok: output.status.success(),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    };
    // The registry's packages hold only their elm.json, so a module that
    // checks cleanly uses none of their values.
    fs::write(
        work.path().join("src/Main.elm"),
        "module Main exposing (x)\n\nimport Html\n\n\nx =\n    1\n",
    )
    .unwrap();
    let Ran { ok, stdout, stderr } = check(&["src/Main.elm"]);
    assert!(ok, "{stdout}{stderr}");
    assert!(stdout.starts_with("No problems in syntax"), "{stdout}");

    fs::write(
        work.path().join("src/Main.elm"),
        "module Main exposing (main)\n\nimport Html\nimport Htm.Attributes\n\n\nmain =\n    Html.text \"hi\"\n",
    )
    .unwrap();
    let Ran { ok, stdout, stderr } = check(&["src/Main.elm"]);
    assert!(!ok);
    assert_eq!(stdout, "Detected problems in 1 module.\n");
    let bar = stderr.lines().next().unwrap();
    assert!(bar.starts_with("-- MODULE NOT FOUND ---"), "{stderr}");
    assert!(
        bar.ends_with(&format!(
            "--- {}",
            Path::new("src").join("Main.elm").display()
        )) && bar.chars().count() == 80,
        "{bar}"
    );
    assert!(stderr.contains("Html.Attributes"), "{stderr}");
    let Ran { ok, stdout, stderr } = check(&["src/Main.elm", "--report", "json"]);
    assert!(!ok);
    assert_eq!(stdout, "");
    assert!(
        stderr.starts_with("{\"type\":\"compile-errors\",\"errors\":[{\"path\":"),
        "{stderr}"
    );

    let Ran { ok, stderr, .. } = check(&[]);
    assert!(!ok);
    assert!(stderr.starts_with("-- NO INPUT"), "{stderr}");
    assert!(stderr.contains("nomi check src/Main.elm"), "{stderr}");
}

/// What a command did: whether it succeeded, and what it printed.
struct Ran {
    ok: bool,
    stdout: String,
    stderr: String,
}
