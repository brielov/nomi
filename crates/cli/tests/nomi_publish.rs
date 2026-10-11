//! `elm publish` end to end (plan 8.14d), against a fixture website: a
//! new package, committed and tagged in git, checked and registered. The
//! server stands in for package.elm-lang.org, GitHub's API, which names
//! the tag's commit, and GitHub, which serves the tag's zipball; it keeps
//! the registration, which the test reads part by part.
#![cfg(unix)]

#[path = "support/registry.rs"]
mod registry;
#[path = "support/server.rs"]
mod server;
#[path = "support/website.rs"]
mod website;

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use server::Server;
use sha1::{Digest, Sha1};

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
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("elm.json"), ELM_JSON).unwrap();
    fs::write(root.join("src/Greet.elm"), GREET).unwrap();
    let readme = format!(
        "# Greet\n\n{}\n",
        "Greet says hello to whoever you name. ".repeat(10)
    );
    fs::write(root.join("README.md"), &readme).unwrap();
    fs::write(root.join("LICENSE"), "BSD 3-Clause License\n").unwrap();
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(&root)
            .env("GIT_CEILING_DIRECTORIES", work.path())
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
    let commit = git(&["rev-list", "-n", "1", "1.0.0"]).trim().to_owned();
    let zipball = zipball(&root);
    let zip_hash: String = Sha1::digest(&zipball)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/elm_publish");
    let website = registry::registry(&fixtures, "0.19.3");
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
                *kept.lock().unwrap() = Some((p.to_owned(), body.to_vec()));
                Some(Vec::new())
            }
            _ => website(method, path, body),
        },
    ));

    let cache = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    let elm = bin.path().join("elm");
    fs::copy(env!("CARGO_BIN_EXE_nomi"), &elm).unwrap();
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
    let marks = |line: &str, waiting: &str, done: &str| {
        let pad = waiting.chars().count().saturating_sub(done.chars().count());
        format!("  → {waiting}\r  ● {done}{}\n{line}", " ".repeat(pad))
    };
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
    let lines: String = checks.iter().map(|(w, d)| marks("", w, d)).collect();
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

    let (path, body) = registered.lock().unwrap().clone().expect("a registration");
    assert_eq!(
        path,
        format!("/register?name=nomi-test/greet&version=1.0.0&commit-hash={commit}")
    );
    let body = String::from_utf8(body).unwrap();
    let part = |name: &str| -> String {
        let start = body
            .find(&format!("name=\"{name}\""))
            .unwrap_or_else(|| panic!("no {name} part"));
        let content = start + body[start..].find("\r\n\r\n").unwrap() + 4;
        let end = content + body[content..].find("\r\n--").unwrap();
        body[content..end].to_owned()
    };
    assert_eq!(part("elm.json"), ELM_JSON);
    assert_eq!(part("README.md"), readme);
    assert_eq!(part("github-hash"), zip_hash);
    assert!(part("docs.json").starts_with(r#"[{"name":"Greet","comment":" Greetings."#));
}

/// A registration the server kept: its path and query, and its body.
type Registration = (String, Vec<u8>);

/// The zipball GitHub would serve of `root`: its files under one top
/// directory.
fn zipball(root: &Path) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    zip.add_directory("nomi-test-greet-0000000/", options)
        .unwrap();
    for file in ["elm.json", "README.md", "LICENSE", "src/Greet.elm"] {
        zip.start_file(format!("nomi-test-greet-0000000/{file}"), options)
            .unwrap();
        zip.write_all(&fs::read(root.join(file)).unwrap()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
