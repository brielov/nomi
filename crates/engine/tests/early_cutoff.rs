//! Early cutoff (plan 6.15): with the artifact cache, a warm check
//! re-checks a module only where its source or its imports' interfaces
//! changed. An edit to a body leaves the module's interface as it was,
//! so its importers are skipped. Optimizing, as `elm make` does, files
//! each module's graph beside its interface (plan 8.12a).
#![cfg(unix)]

#[path = "support/projects.rs"]
mod projects;

use std::fs;
use std::path::{Path, PathBuf};

use nomi_base::Compat;
use nomi_compiler::optimized::binary;
use nomi_engine::Vfs;
use nomi_engine::artifacts::Artifacts;
use nomi_engine::check::{self, Checked};
use nomi_engine::console::Personality;
use nomi_engine::graph::Foreigns;
use nomi_engine::names;
use nomi_engine::paths::Cache;
use nomi_project::{Name, Outline};

const ELM_JSON: &str = r#"{
    "type": "application",
    "source-directories": ["src"],
    "elm-version": "0.19.1",
    "dependencies": { "direct": { "elm/core": "1.0.5" }, "indirect": {} },
    "test-dependencies": { "direct": {}, "indirect": {} }
}
"#;

const MAIN: &str =
    "module Main exposing (value)\n\nimport Middle\n\n\nvalue : Int\nvalue =\n    Middle.twice 2\n";
const MIDDLE: &str = "module Middle exposing (twice)\n\nimport Base\n\n\ntwice : Int -> Int\ntwice n =\n    Base.add n n\n";
const BASE: &str =
    "module Base exposing (add)\n\n\nadd : Int -> Int -> Int\nadd a b =\n    a + b\n";

struct Project {
    _dir: tempfile::TempDir,
    _cache: tempfile::TempDir,
    root: PathBuf,
    artifacts: Artifacts,
}

impl Project {
    fn new() -> Project {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        fs::write(root.join("elm.json"), ELM_JSON).unwrap();
        fs::create_dir(root.join("src")).unwrap();
        for (name, text) in [("Main", MAIN), ("Middle", MIDDLE), ("Base", BASE)] {
            fs::write(root.join(format!("src/{name}.elm")), text).unwrap();
        }
        let cache = tempfile::tempdir().unwrap();
        let artifacts = Artifacts::new(&Cache::new(cache.path().to_owned()));
        Project {
            _dir: dir,
            _cache: cache,
            root,
            artifacts,
        }
    }

    fn write(&self, name: &str, text: &str) {
        fs::write(self.root.join(format!("src/{name}.elm")), text).unwrap();
    }

    /// Checks the project, and names the modules checked rather than
    /// reused, sorted.
    fn check(&self) -> Vec<String> {
        self.build(false).0
    }

    /// Optimizes the project as `elm make` does: the modules checked
    /// rather than reused, sorted, and every module's graph, encoded.
    fn optimize(&self) -> (Vec<String>, Vec<(String, Vec<u8>)>) {
        self.build(true)
    }

    fn build(&self, optimize: bool) -> (Vec<String>, Vec<(String, Vec<u8>)>) {
        let compat = Compat::V0_19_1;
        let packages = Path::new(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/packages");
        let outline = Outline::decode(ELM_JSON.as_bytes(), compat).unwrap();
        let core = Name::core();
        let foreigns =
            Foreigns::new([(core.clone(), projects::exposing(&packages, &core, compat))]);
        let interfaces = names::visible(&projects::interfaces(&packages, compat), &[core]);
        let env = check::env_of(&self.root, &outline, compat, foreigns)
            .unwrap()
            .with_cwd(&self.root);
        let snapshot = Vfs::new().snapshot();
        let paths = [PathBuf::from("src/Main.elm")];
        let artifacts = Some(&self.artifacts);
        let (checked, ran, graphs) = if optimize {
            let (checked, optimized) = check::optimize(
                &snapshot,
                &env,
                &outline,
                &paths,
                Personality::Nomi,
                (&interfaces).into(),
                artifacts,
            );
            let mut graphs: Vec<(String, Vec<u8>)> = (optimized.graphs.iter())
                .map(|(name, graph)| (name.to_string(), binary::encode_local_graph(graph, compat)))
                .collect();
            graphs.sort();
            (checked, optimized.ran, graphs)
        } else {
            let (checked, ran) = check::check_with(
                &snapshot,
                &env,
                &outline,
                &paths,
                Personality::Nomi,
                (&interfaces).into(),
                artifacts,
            );
            (checked, ran, Vec::new())
        };
        assert!(matches!(checked, Checked::Fine), "{checked:?}");
        let mut names: Vec<String> = (ran.iter())
            .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        (names, graphs)
    }
}

#[test]
fn a_body_edit_rechecks_one_module() {
    let project = Project::new();
    assert_eq!(project.check(), ["Base", "Main", "Middle"]);
    // Warm, nothing changed: nothing is checked.
    assert!(project.check().is_empty());
    // A body edit in the module everything rests on: its interface is
    // as it was, so neither importer is checked again.
    project.write("Base", &BASE.replace("a + b", "b + a"));
    assert_eq!(project.check(), ["Base"]);
    // A changed signature changes the interface, and its importer is
    // checked again; that importer's interface is the same, so the cutoff
    // stops there.
    project.write(
        "Base",
        "module Base exposing (add)\n\n\nadd : number -> number -> number\nadd a b =\n    a + b\n",
    );
    assert_eq!(project.check(), ["Base", "Middle"]);
}

#[test]
fn optimized_graphs_are_reused() {
    let project = Project::new();
    // A check files interfaces only, so the first build checks again.
    assert_eq!(project.check(), ["Base", "Main", "Middle"]);
    let (ran, cold) = project.optimize();
    assert_eq!(ran, ["Base", "Main", "Middle"]);
    assert_eq!(cold.len(), 3);
    // Warm: every graph comes from the cache, the same.
    let (ran, warm) = project.optimize();
    assert!(ran.is_empty());
    assert_eq!(warm, cold);
    // A body edit rebuilds the one module, and its graph alone changes.
    project.write("Base", &BASE.replace("a + b", "b + a"));
    let (ran, edited) = project.optimize();
    assert_eq!(ran, ["Base"]);
    let changed: Vec<&str> = (edited.iter().zip(&cold))
        .filter(|(a, b)| a != b)
        .map(|(a, _)| a.0.as_str())
        .collect();
    assert_eq!(changed, ["Base"]);
    // And a check after builds reuses everything too.
    assert!(project.check().is_empty());
}

#[test]
fn documenting_reuses_the_modules_it_does_not_document() {
    const PKG_JSON: &str = r#"{
    "type": "package",
    "name": "author/pkg",
    "summary": "a package",
    "license": "BSD-3-Clause",
    "version": "1.0.0",
    "exposed-modules": ["Main"],
    "elm-version": "0.19.0 <= v < 0.20.0",
    "dependencies": { "elm/core": "1.0.0 <= v < 2.0.0" },
    "test-dependencies": {}
}
"#;
    let project = Project::new();
    let main = "module Main exposing (value)\n\n{-| A module.\n\n@docs value\n\n-}\n\nimport Middle\n\n\n{-| A value.\n-}\nvalue : Int\nvalue =\n    Middle.twice 2\n";
    project.write("Main", main);
    fs::write(project.root.join("elm.json"), PKG_JSON).unwrap();
    let compat = Compat::V0_19_1;
    let packages = Path::new(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/packages");
    let outline = Outline::decode(PKG_JSON.as_bytes(), compat).unwrap();
    let core = Name::core();
    let foreigns = Foreigns::new([(core.clone(), projects::exposing(&packages, &core, compat))]);
    let interfaces = names::visible(&projects::interfaces(&packages, compat), &[core]);
    let env = check::env_of(&project.root, &outline, compat, foreigns)
        .unwrap()
        .with_cwd(&project.root);
    let docs = |artifacts| {
        let snapshot = Vfs::new().snapshot();
        let (checked, json, _) = check::docs_with(
            &snapshot,
            &env,
            &outline,
            Personality::Elm,
            (&interfaces).into(),
            artifacts,
        );
        assert!(matches!(checked, Checked::Fine), "{checked:?}");
        json.unwrap()
    };
    let fresh = docs(None);
    // Cold, then warm with Middle and Base read back.
    assert_eq!(docs(Some(&project.artifacts)), fresh);
    assert_eq!(docs(Some(&project.artifacts)), fresh);
    assert!(fresh.contains("A value."), "{fresh}");
}
