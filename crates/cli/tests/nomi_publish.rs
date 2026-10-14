//! `elm publish` end to end, against a fixture website: a
//! new package, committed and tagged in git, checked and registered. The
//! server stands in for package.elm-lang.org, GitHub's API, which names
//! the tag's commit, and GitHub, which serves the tag's zipball; it keeps
//! the registration, which the test reads part by part.
#![cfg(unix)]

#[path = "support/registry.rs"]
pub mod registry;
#[path = "support/server.rs"]
pub mod server;
#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use nomi_base::Compat;
use server::Server;

const ELM_JSON: &str = r#"{
    "type": "package",
    "name": "nomi-test/greet",
    "summary": "Greetings, as a test publishes them",
    "license": "BSD-3-Clause",
    "version": "1.0.0",
    "exposed-modules": [
        "Greet"
    ],
    "elm-version": "0.19.0 <= v < 0.20.0",
    "dependencies": {
        "elm/core": "1.0.0 <= v < 2.0.0"
    },
    "test-dependencies": {}
}
"#;

const GREET: &str = "module Greet exposing (hello)

{-| Greetings.

@docs hello

-}


{-| Say hello.
-}
hello : String -> String
hello name =
    \"Hello, \" ++ name ++ \"!\"
";

#[test]
fn a_tagged_package_publishes() {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap().join("greet");
    let readme = format!(
        "# Greet\n\n{}\n",
        "Greet says hello to whoever you name. ".repeat(10)
    );
    let commit = tagged_package(&root, work.path(), &readme);
    // The zipball GitHub would serve of the tag: its files under one top
    // directory.
    let files = ["elm.json", "README.md", "LICENSE", "src/Greet.elm"];
    let zipball = website::zipped("nomi-test-greet-0000000", &root, &files);
    let zip_hash = nomi_registry::store::sha1(&zipball);

    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/elm_publish");
    let website = registry::registry(&fixtures, Compat::LATEST);
    let registered: Arc<Mutex<Option<Registration>>> = Arc::default();
    let kept = registered.clone();
    let sha = commit.clone();
    let server = Server::start(Arc::new(
        move |method: &str, path: &str, body: &[u8]| match (method, path) {
            ("GET", "/github-api/repos/nomi-test/greet/git/refs/tags/1.0.0") => {
                Some(format!(r#"{{"object":{{"sha":"{sha}","type":"commit"}}}}"#).into_bytes())
            }
            ("GET", "/github/nomi-test/greet/zipball/1.0.0/") => Some(zipball.clone()),
            ("POST", p) if p.starts_with("/register?") => {
                *kept.lock().unwrap() = Some(Registration {
                    path: p.to_owned(),
                    body: body.to_vec(),
                });
                Some(Vec::new())
            }
            _ => website(method, path, body),
        },
    ));

    let cache = tempfile::tempdir().unwrap();
    let (_bin, elm) = website::personality("elm");
    let output = website::offline(&elm, cache.path())
        .arg("publish")
        .current_dir(&root)
        .env("NOMI_REGISTRY", &server.url)
        .env_remove("ALL_PROXY")
        .env("GIT_CEILING_DIRECTORIES", work.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "{stdout}{stderr}");
    let lines = checks();
    assert!(
        stdout.starts_with("This package has never been published before."),
        "{stdout}"
    );
    assert!(
        stdout.ends_with(&format!(
            "I will now verify that everything is in order...\n\n{lines}\nSuccess!\n"
        )),
        "{stdout}"
    );

    let Registration { path, body } = registered.lock().unwrap().take().expect("a registration");
    assert_eq!(
        path,
        format!("/register?name=nomi-test/greet&version=1.0.0&commit-hash={commit}")
    );
    let body = String::from_utf8(body).unwrap();
    assert_eq!(part(&body, "elm.json"), ELM_JSON);
    assert_eq!(part(&body, "README.md"), readme);
    assert_eq!(part(&body, "github-hash"), zip_hash);
    assert!(part(&body, "docs.json").starts_with(r#"[{"name":"Greet","comment":" Greetings."#));
}

/// Writes the package to `root`, a git repository of its own below
/// `ceiling`, committed and tagged 1.0.0; the tag's commit.
fn tagged_package(root: &Path, ceiling: &Path, readme: &str) -> String {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("elm.json"), ELM_JSON).unwrap();
    fs::write(root.join("src/Greet.elm"), GREET).unwrap();
    fs::write(root.join("README.md"), readme).unwrap();
    fs::write(root.join("LICENSE"), "BSD 3-Clause License\n").unwrap();
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(root)
            .env("GIT_CEILING_DIRECTORIES", ceiling)
            .env("GIT_AUTHOR_NAME", "nomi")
            .env("GIT_AUTHOR_EMAIL", "nomi@example.com")
            .env("GIT_COMMITTER_NAME", "nomi")
            .env("GIT_COMMITTER_EMAIL", "nomi@example.com")
            .output()
            .unwrap();
        assert!(status.status.success(), "git {args:?}: {status:?}");
        String::from_utf8(status.stdout).unwrap()
    };
    git(&["init", "--quiet"]);
    git(&["add", "."]);
    git(&["commit", "--quiet", "-m", "first"]);
    git(&["tag", "-a", "1.0.0", "-m", "new release"]);
    git(&["rev-list", "-n", "1", "1.0.0"]).trim().to_owned()
}

/// The checks `elm publish` prints, each one's waiting line overwritten
/// by its outcome.
fn checks() -> String {
    let checks = [
        ("Looking for README.md", "Found README.md"),
        ("Looking for LICENSE", "Found LICENSE"),
        ("Verifying documentation...", "Verified documentation"),
        (
            "Checking semantic versioning rules. Is 1.0.0 correct?",
            "All packages start at version 1.0.0",
        ),
        (
            "Is version 1.0.0 tagged on GitHub?",
            "Version 1.0.0 is tagged on GitHub",
        ),
        (
            "Checking for uncommitted changes...",
            "No uncommitted changes in local code",
        ),
        (
            "Downloading code from GitHub...",
            "Code downloaded successfully from GitHub",
        ),
        (
            "Verifying downloaded code...",
            "Downloaded code compiles successfully",
        ),
    ];
    let marks = |(waiting, done): &(&str, &str)| {
        let pad = waiting.chars().count().saturating_sub(done.chars().count());
        format!("  → {waiting}\r  ● {done}{}\n", " ".repeat(pad))
    };
    checks.iter().map(marks).collect()
}

/// The content of the multipart form `body`'s part `name`.
fn part<'b>(body: &'b str, name: &str) -> &'b str {
    let start = body
        .find(&format!("name=\"{name}\""))
        .unwrap_or_else(|| panic!("no {name} part"));
    let content = start + body[start..].find("\r\n\r\n").unwrap() + 4;
    let end = content + body[content..].find("\r\n--").unwrap();
    &body[content..end]
}

/// A registration the server kept.
struct Registration {
    /// Its path and query.
    path: String,
    body: Vec<u8>,
}
