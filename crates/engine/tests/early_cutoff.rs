//! Early cutoff (plan 6.15): with the artifact cache, a warm check
//! re-checks a module only where its source or its imports' interfaces
//! changed. An edit to a body leaves the module's interface as it was,
//! so its importers are skipped.
#![cfg(unix)]

#[path = "support/projects.rs"]
mod projects;

use std::fs;
use std::path::{Path, PathBuf};

use nomi_base::Compat;
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
    "module Main exposing (main)\n\nimport Middle\n\n\nmain : Int\nmain =\n    Middle.twice 2\n";
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
        let (checked, ran) = check::check_with(
            &Vfs::new().snapshot(),
            &env,
            &outline,
            &[PathBuf::from("src/Main.elm")],
            Personality::Nomi,
            &interfaces,
            Some(&self.artifacts),
        );
        assert!(matches!(checked, Checked::Fine), "{checked:?}");
        let mut names: Vec<String> = (ran.iter())
            .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
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
