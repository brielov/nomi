//! A workspace built from its root: every member in order, each entry
//! once, the member package built once for both applications, and the
//! artifact cache shared, so that a build after an edit checks only the
//! modules the edit reaches.
#![cfg(unix)]

use std::fs;
use std::path::PathBuf;

use nomi_engine::console::Captured;
use nomi_engine::javascript::Kind;
use nomi_engine::paths::Cache;
use nomi_engine::workspace::{self, Making, Outcome};
use nomi_engine::{Context, Vfs};
use nomi_registry::{Http, HttpError};
use nomi_testkit::{packages, write_file};

/// The package website, offline but for its list of the fixture
/// packages, which the cache already holds.
struct Offline;

impl Http for Offline {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        Err(HttpError::Transport(format!("offline: {url}")))
    }

    fn post(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        if url.ends_with("/all-packages") {
            Ok(packages::all_packages())
        } else {
            Err(HttpError::Transport(format!("offline: {url}")))
        }
    }
}

const APP: &str = r#"{
    "type": "application",
    "source-directories": ["src"],
    "elm-version": "0.19.1",
    "dependencies": {
        "direct": { "acme/shared": "1.0.0", "elm/core": "1.0.5", "elm/json": "1.1.4" },
        "indirect": {}
    },
    "test-dependencies": { "direct": {}, "indirect": {} }
}
"#;

const SHARED: &str = r#"{
    "type": "package",
    "name": "acme/shared",
    "summary": "shared",
    "license": "BSD-3-Clause",
    "version": "1.0.0",
    "exposed-modules": ["Shared", "Shared.Other"],
    "elm-version": "0.19.0 <= v < 0.20.0",
    "dependencies": { "elm/core": "1.0.0 <= v < 2.0.0" },
    "test-dependencies": {}
}
"#;

/// A worker whose model is `value`, a module importing `imports`.
fn worker(module: &str, imports: &[&str], value: &str) -> String {
    let imports: String = imports.iter().map(|i| format!("import {i}\n")).collect();
    format!(
        "module {module} exposing (main)\n\n{imports}\n\nmain : Program () Int ()\nmain =\n    \
         Platform.worker\n        {{ init = \\() -> ( {value}, Cmd.none )\n        \
         , update = \\() n -> ( n, Cmd.none )\n        , subscriptions = \\_ -> Sub.none\n        \
         }}\n"
    )
}

/// The module `name` exposing the `Int` `value`, `body` after importing
/// `imports`.
fn value(name: &str, imports: &[&str], value: &str, body: &str) -> String {
    let imports: String = imports.iter().map(|i| format!("import {i}\n")).collect();
    format!(
        "module {name} exposing ({value})\n\n{imports}\n\n{value} : Int\n{value} =\n    {body}\n"
    )
}

/// `acme/shared`'s `Shared.Base`, whose `base` is `body`.
fn base(body: &str) -> String {
    value("Shared.Base", &[], "base", body)
}

/// A workspace of `apps/web`, with two browser entries, `apps/api`, with
/// one for Node, and the package `packages/shared` both use; in each
/// application, one module that imports the package and one that doesn't.
struct Fixture {
    _work: tempfile::TempDir,
    _cache: tempfile::TempDir,
    root: PathBuf,
    cache: Cache,
}

impl Fixture {
    fn new() -> Fixture {
        let work = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(work.path()).unwrap();
        let shared = "Shared.Base.base + Shared.Other.other";
        let files = [
            (
                "nomi.toml",
                "[workspace]\nmembers = [\"apps/*\", \"packages/*\"]\n".to_owned(),
            ),
            ("packages/shared/elm.json", SHARED.to_owned()),
            (
                "packages/shared/src/Shared.elm",
                value("Shared", &["Shared.Base", "Shared.Other"], "shared", shared),
            ),
            ("packages/shared/src/Shared/Base.elm", base("1")),
            (
                "packages/shared/src/Shared/Other.elm",
                value("Shared.Other", &[], "other", "2"),
            ),
            ("apps/web/elm.json", APP.to_owned()),
            (
                "apps/web/nomi.toml",
                "[[entry]]\nmain = \"src/Main.elm\"\noutput = \"web.js\"\n\n\
                 [[entry]]\nmain = \"src/Admin.elm\"\nplatform = \"browser\"\n"
                    .to_owned(),
            ),
            (
                "apps/web/src/Main.elm",
                worker(
                    "Main",
                    &["Web.View", "Web.Plain"],
                    "Web.View.view + Web.Plain.plain",
                ),
            ),
            (
                "apps/web/src/Admin.elm",
                worker("Admin", &["Web.View"], "Web.View.view"),
            ),
            (
                "apps/web/src/Web/View.elm",
                value("Web.View", &["Shared"], "view", "Shared.shared"),
            ),
            (
                "apps/web/src/Web/Plain.elm",
                value("Web.Plain", &[], "plain", "3"),
            ),
            ("apps/api/elm.json", APP.to_owned()),
            (
                "apps/api/nomi.toml",
                "[[entry]]\nmain = \"src/Main.elm\"\nplatform = \"node\"\n".to_owned(),
            ),
            (
                "apps/api/src/Main.elm",
                worker(
                    "Main",
                    &["Shared", "Api.Plain"],
                    "Shared.shared + Api.Plain.plain",
                ),
            ),
            (
                "apps/api/src/Api/Plain.elm",
                value("Api.Plain", &[], "plain", "4"),
            ),
        ];
        for (path, text) in files {
            write_file(&root, path, &text);
        }
        let cache = tempfile::tempdir().unwrap();
        packages::store(&cache.path().join("packages")).unwrap();
        Fixture {
            _work: work,
            root,
            cache: Cache::new(cache.path().to_owned()),
            _cache: cache,
        }
    }

    fn write(&self, path: &str, text: &str) {
        write_file(&self.root, path, text);
    }

    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.root.join(path)).unwrap()
    }

    /// Builds the workspace from its root: what was shown, and how each
    /// member went, by its directory, the modules it checked by their
    /// paths from the root, sorted.
    fn build(&self) -> (Captured, Vec<(String, Outcome)>) {
        let vfs = Vfs::new();
        let cx = Context {
            vfs: &vfs,
            dir: &self.root,
            cache: &self.cache,
            http: &Offline,
        };
        let workspace = workspace::discover(&vfs.snapshot(), &self.root)
            .unwrap()
            .unwrap();
        let mut console = Captured::default();
        let every: Vec<PathBuf> = (workspace.members().iter())
            .map(|member| member.dir.clone())
            .collect();
        let outcomes = workspace::build(
            &cx,
            &mut console,
            &workspace,
            &every,
            Making::Code(Kind::Dev),
        );
        let outcomes = (outcomes.into_iter())
            .map(|(member, outcome)| {
                let outcome = match outcome {
                    Outcome::Built(ran) => Outcome::Built(self.relative(&ran)),
                    outcome => outcome,
                };
                (member.dir.display().to_string(), outcome)
            })
            .collect();
        (console, outcomes)
    }

    /// `ran`, by their paths from the root, sorted.
    fn relative(&self, ran: &[PathBuf]) -> Vec<PathBuf> {
        let mut ran: Vec<PathBuf> = (ran.iter())
            .map(|path| path.strip_prefix(&self.root).unwrap().to_owned())
            .collect();
        ran.sort();
        ran
    }
}

/// How the members went, each built and checking `ran`: shared, then the
/// applications in order of their paths.
fn built(shared: &[&str], api: &[&str], web: &[&str]) -> Vec<(String, Outcome)> {
    let paths = |paths: &[&str]| Outcome::Built(paths.iter().map(PathBuf::from).collect());
    vec![
        ("packages/shared".to_owned(), paths(shared)),
        ("apps/api".to_owned(), paths(api)),
        ("apps/web".to_owned(), paths(web)),
    ]
}

fn said(console: &Captured) -> String {
    format!("{}{}", console.out, console.err)
}

#[test]
fn every_entry_builds_and_a_second_build_reuses_everything() {
    let fixture = Fixture::new();
    let (console, outcomes) = fixture.build();
    assert_eq!(
        outcomes,
        built(
            &[
                "packages/shared/src/Shared/Base.elm",
                "packages/shared/src/Shared/Other.elm",
                "packages/shared/src/Shared.elm",
            ],
            &["apps/api/src/Api/Plain.elm", "apps/api/src/Main.elm"],
            &[
                "apps/web/src/Admin.elm",
                "apps/web/src/Main.elm",
                "apps/web/src/Web/Plain.elm",
                "apps/web/src/Web/View.elm",
            ],
        ),
        "{}",
        said(&console)
    );
    let web = fixture.read("apps/web/web.js");
    assert!(
        web.contains("var $acme$shared$Shared$Base$base = 1;"),
        "{web}"
    );
    let admin = fixture.read("apps/web/elm-stuff/nomi/run/src/Admin.js");
    assert!(
        admin.contains("var $author$project$Admin$main = "),
        "{admin}"
    );
    let api = fixture.read("apps/api/elm-stuff/nomi/run/src/Main.mjs");
    assert!(api.contains("var $acme$shared$Shared$shared = "), "{api}");
    let (console, outcomes) = fixture.build();
    assert_eq!(outcomes, built(&[], &[], &[]), "{}", said(&console));
}

#[test]
fn an_edit_in_the_package_rebuilds_what_depends_on_it_and_no_more() {
    let fixture = Fixture::new();
    fixture.build();
    // A body: the module alone, and every entry written again with it.
    fixture.write("packages/shared/src/Shared/Base.elm", &base("5"));
    let (console, outcomes) = fixture.build();
    assert_eq!(
        outcomes,
        built(&["packages/shared/src/Shared/Base.elm"], &[], &[]),
        "{}",
        said(&console)
    );
    let web = fixture.read("apps/web/web.js");
    assert!(
        web.contains("var $acme$shared$Shared$Base$base = 5;"),
        "{web}"
    );
    let api = fixture.read("apps/api/elm-stuff/nomi/run/src/Main.mjs");
    assert!(
        api.contains("var $acme$shared$Shared$Base$base = 5;"),
        "{api}"
    );
    // An exposed interface: the modules importing it in each application,
    // once each, but not what imports those, whose interfaces stay.
    let shared = fixture.read("packages/shared/src/Shared.elm");
    let shared = shared.replace("exposing (shared)", "exposing (shared, extra)")
        + "\n\nextra : Int\nextra =\n    0\n";
    fixture.write("packages/shared/src/Shared.elm", &shared);
    let (console, outcomes) = fixture.build();
    assert_eq!(
        outcomes,
        built(
            &["packages/shared/src/Shared.elm"],
            &["apps/api/src/Main.elm"],
            &["apps/web/src/Web/View.elm"],
        ),
        "{}",
        said(&console)
    );
}

#[test]
fn a_failed_package_blocks_its_dependents_which_say_nothing() {
    let fixture = Fixture::new();
    fixture.write("packages/shared/src/Shared/Base.elm", &base("\"one\""));
    let (console, outcomes) = fixture.build();
    let blocked = |dir: &str| (dir.to_owned(), Outcome::Blocked);
    assert_eq!(
        outcomes,
        vec![
            ("packages/shared".to_owned(), Outcome::Failed),
            blocked("apps/api"),
            blocked("apps/web"),
        ],
        "{}",
        said(&console)
    );
    let bars: Vec<&str> = (console.err.lines())
        .filter(|line| line.starts_with("-- "))
        .collect();
    assert_eq!(bars.len(), 1, "{}", said(&console));
    assert!(
        bars[0].starts_with("-- TYPE MISMATCH -")
            && bars[0].ends_with("- packages/shared/src/Shared/Base.elm"),
        "{}",
        said(&console)
    );
    assert!(!fixture.root.join("apps/web/web.js").exists());
}

#[test]
fn an_error_in_an_application_is_shown_from_the_root() {
    let fixture = Fixture::new();
    fixture.write(
        "apps/web/src/Web/Plain.elm",
        &value("Web.Plain", &[], "plain", "\"three\""),
    );
    let (console, outcomes) = fixture.build();
    let members: Vec<(&str, bool)> = (outcomes.iter())
        .map(|(dir, outcome)| (dir.as_str(), matches!(outcome, Outcome::Built(_))))
        .collect();
    assert_eq!(
        members,
        [
            ("packages/shared", true),
            ("apps/api", true),
            ("apps/web", false)
        ],
        "{}",
        said(&console)
    );
    let bar = console
        .err
        .lines()
        .find(|l| l.starts_with("-- TYPE MISMATCH -"));
    assert!(
        bar.is_some_and(|bar| bar.ends_with("- apps/web/src/Web/Plain.elm")),
        "{}",
        said(&console)
    );
}

#[test]
fn an_entry_written_where_no_code_goes_is_refused() {
    let fixture = Fixture::new();
    fixture.write(
        "apps/api/nomi.toml",
        "[[entry]]\nmain = \"src/Main.elm\"\noutput = \"api.txt\"\n",
    );
    let (console, outcomes) = fixture.build();
    assert_eq!(outcomes[1], ("apps/api".to_owned(), Outcome::Failed));
    assert!(console.err.contains("-- BAD OUTPUT"), "{}", said(&console));
    assert!(
        !fixture.root.join("apps/api/api.txt").exists(),
        "{}",
        said(&console)
    );
}
