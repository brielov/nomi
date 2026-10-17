//! What the editor checks in each project with an open file, as
//! elm-language-server does: elm's problems, or each open file's syntax
//! error where elm can't check the project; elm-review's, run on open
//! and save once elm finds nothing; and in the open files the project's
//! `elm-analyse.json` doesn't exclude, the lint rules' problems and the
//! annotations left out.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nomi_base::ColumnUnit;
use nomi_engine::Snapshot;
use nomi_engine::index::{Index, Owners};
use nomi_engine::live::Live;
use nomi_engine::schedule::Cancelled;
use serde_json::Value;

use crate::problems::slashed;
use crate::review::{self, ReviewSettings};
use crate::{
    Checks, Code, Problem, Rule, compiler_problems, diagnostics, lint, missing_annotations,
};

/// What the client's `elmLS` settings say to check.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckSettings {
    /// `onlyUpdateDiagnosticsOnSave`: only opening and saving check, and
    /// annotations aren't suggested.
    pub on_save_only: bool,
    /// `disableElmLSDiagnostics`.
    pub no_lint: bool,
    /// How elm-review runs, where the client turned it on.
    pub review: Option<ReviewSettings>,
}

impl CheckSettings {
    /// Whether the lint rules run. Diagnostics on save run them on open
    /// and save whatever `disableElmLSDiagnostics` says, as
    /// elm-language-server does.
    fn lint(&self) -> bool {
        self.on_save_only || !self.no_lint
    }
}

/// elm-review's problems by file, from the last open or save that ran
/// it, less those of the files changed since.
#[derive(Debug, Default)]
pub struct Reviewed(HashMap<PathBuf, Vec<Problem>>);

impl Reviewed {
    /// Forgets the problems of the file at `path`, which the editor
    /// changed.
    pub fn changed(&mut self, path: &Path) {
        self.0.remove(path);
    }
}

/// A project's `elm-analyse.json`: which lint rules run, and which files
/// have none.
struct Analyse {
    checks: Checks,
    excluded: Vec<PathBuf>,
}

impl Analyse {
    fn read(snapshot: &Snapshot, root: &Path) -> Analyse {
        let json = (snapshot.read(&root.join("elm-analyse.json")).ok())
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .unwrap_or(Value::Null);
        let checks = (json.get("checks").and_then(Value::as_object).into_iter())
            .flatten()
            .filter_map(|(name, on)| Some((Rule::from_name(name)?, on.as_bool()?)));
        let excluded = (json
            .get("excludedPaths")
            .and_then(Value::as_array)
            .into_iter())
        .flatten()
        .filter_map(Value::as_str)
        .map(|path| root.join(path))
        .collect();
        Analyse {
            checks: Checks::new(checks),
            excluded,
        }
    }

    fn excludes(&self, path: &Path) -> bool {
        self.excluded.iter().any(|dir| path.starts_with(dir))
    }
}

/// What the editor checks in a project, as `settings` and its
/// `elm-analyse.json` have it.
struct Plan<'o> {
    /// The open files `elm-analyse.json` doesn't exclude, each with its
    /// text where the lint rules run and it reads.
    linted: Vec<(&'o Path, Option<String>)>,
    /// Whether the annotations left out are suggested.
    annotations: bool,
    /// The file to index the project from, where the problems need an
    /// index.
    index_from: Option<&'o Path>,
}

impl<'o> Plan<'o> {
    /// The plan for a project of the `open` files, `read` reading a file's
    /// text.
    fn of(
        settings: &CheckSettings,
        analyse: &Analyse,
        open: &'o [PathBuf],
        read: impl Fn(&Path) -> Option<String>,
    ) -> Plan<'o> {
        let linted: Vec<(&Path, Option<String>)> = (open.iter())
            .filter(|p| !analyse.excludes(p))
            .map(|p| (p.as_path(), settings.lint().then(|| read(p)).flatten()))
            .collect();
        let annotations = !settings.on_save_only && analyse.checks.on(Rule::MissingTypeAnnotation);
        // Indexing takes a while; only annotations and ports need it.
        let ports =
            (linted.iter()).any(|(_, text)| text.as_ref().is_some_and(|t| t.contains("port ")));
        let index_from = (annotations || ports)
            .then(|| linted.first().map(|(p, _)| *p))
            .flatten();
        Plan {
            linted,
            annotations,
            index_from,
        }
    }
}

/// What becomes of a project's elm-review problems once elm checked it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Review {
    Keep,
    Forget,
    /// Forget them and run elm-review again.
    Rerun,
}

/// What elm found in a project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Elm {
    Clean,
    Problems,
}

impl Review {
    /// elm-review runs on open and save, once elm finds nothing; its
    /// problems go when elm finds some.
    fn after(saved: bool, elm: Elm) -> Review {
        match (saved, elm) {
            (true, Elm::Clean) => Review::Rerun,
            (false, Elm::Clean) => Review::Keep,
            (_, Elm::Problems) => Review::Forget,
        }
    }
}

/// The problems of every project that owns an `open` file, each file's
/// from its owner alone (R7), as `settings` has them, on open or save if
/// `saved`, with elm-review's kept in `reviewed` and its columns read in
/// `unit`. `check` checks a project, by its manifest
/// and open files, as elm does, and indexes it from the file it's given
/// where the problems need an index; cancelled where it is.
pub fn project_problems(
    snapshot: &Snapshot,
    open: &[PathBuf],
    settings: &CheckSettings,
    saved: bool,
    reviewed: &mut Reviewed,
    unit: ColumnUnit,
    mut check: impl FnMut(
        &Path,
        &[PathBuf],
        Option<&Path>,
    ) -> Result<(Option<Live>, Option<Arc<Index>>), Cancelled>,
) -> Result<Vec<Problem>, Cancelled> {
    let owners = Owners::read(snapshot, open);
    let mut projects: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    let mut found = Vec::new();
    for path in open {
        match owners.owner(snapshot, path) {
            Some(manifest) => projects.entry(manifest).or_default().push(path.clone()),
            None => found.extend(syntax_problems(snapshot, path, None)),
        }
    }
    for (manifest, open) in projects {
        let root = manifest.parent().unwrap_or_else(|| Path::new(""));
        let analyse = Analyse::read(snapshot, root);
        let plan = Plan::of(settings, &analyse, &open, |p| snapshot.read(p).ok());
        let (live, index) = check(&manifest, &open, plan.index_from)?;
        let before = found.len();
        match live {
            // A module another project owns, such as one two projects'
            // source directories list, is that project's to show.
            Some(live) => found.extend(
                (compiler_problems(snapshot, &live, &open).into_iter())
                    .filter(|p| owners.owner(snapshot, &p.path).as_ref() == Some(&manifest)),
            ),
            None => {
                for path in &open {
                    found.extend(syntax_problems(snapshot, path, Some(&manifest)));
                }
            }
        }
        let elm = if found.len() == before {
            Elm::Clean
        } else {
            Elm::Problems
        };
        let review = Review::after(saved, elm);
        if review != Review::Keep {
            reviewed.0.retain(|path, _| !path.starts_with(root));
        }
        if review == Review::Rerun
            && let Some(review) = &settings.review
        {
            for problem in review::problems(snapshot, root, review, unit) {
                (reviewed.0.entry(problem.path.clone()).or_default()).push(problem);
            }
        }
        let mut review_found: Vec<&Problem> = (reviewed.0.iter())
            .filter(|(path, _)| path.starts_with(root))
            .flat_map(|(_, problems)| problems)
            .collect();
        review_found.sort_by(|a, b| (&a.path, a.range.start()).cmp(&(&b.path, b.range.start())));
        found.extend(review_found.into_iter().cloned());
        // The texts are read where the rules run.
        for (path, text) in &plan.linted {
            let Some(text) = text else { continue };
            found.extend(lint(
                snapshot,
                index.as_deref(),
                path,
                text,
                &analyse.checks,
            ));
        }
        if plan.annotations
            && let Some(index) = &index
        {
            for (path, _) in &plan.linted {
                found.extend(missing_annotations(index, path));
            }
        }
    }
    Ok(found)
}

/// The syntax error in the file at `path`, for a file elm can't check
/// with its project, whose `elm.json` is `manifest`: one outside any, or
/// whose dependencies the store doesn't hold. Its data names the file
/// relative to the project, or by its name alone.
fn syntax_problems(snapshot: &Snapshot, path: &Path, manifest: Option<&Path>) -> Vec<Problem> {
    let root = manifest.and_then(Path::parent);
    let relative = || match root.and_then(|root| path.strip_prefix(root).ok()) {
        Some(relative) => slashed(relative),
        None => slashed(Path::new(path.file_name().unwrap_or_default())),
    };
    (diagnostics(snapshot, path, manifest)
        .unwrap_or_default()
        .into_iter())
    .map(|d| Problem {
        relative: Some(relative()),
        ..Problem::new(
            path.to_owned(),
            d.range,
            Code::ElmMake,
            format!("{} - {}", d.title, d.message),
        )
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_read_as_its_owner_reads_it_where_its_owner_cant_check_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = nomi_engine::canonical(dir.path()).unwrap();
        // The package's `Deep` directory is the application's too, which
        // owns it as the deeper once its own file is open; a port module is
        // no package's.
        let package = r#"{ "type": "package", "name": "acme/lib", "summary": "lib",
            "license": "BSD-3-Clause", "version": "1.0.0", "exposed-modules": [],
            "elm-version": "0.19.0 <= v < 0.20.0",
            "dependencies": { "elm/core": "1.0.0 <= v < 2.0.0" },
            "test-dependencies": {} }"#;
        let app = (nomi_testkit::packages::CORE_AND_JSON_APP)
            .replace(r#"["src"]"#, r#"["src", "../lib/src/Deep"]"#);
        nomi_testkit::write_file(&root, "lib/elm.json", package);
        nomi_testkit::write_file(&root, "app/elm.json", &app);
        let ports = "module Ports exposing (out)\n\n\nport out : String -> Cmd msg\n";
        let path = root.join("lib/src/Deep/Ports.elm");
        nomi_testkit::write_file(&root, "lib/src/Deep/Ports.elm", &format!("port {ports}"));
        let main = root.join("app/src/Main.elm");
        nomi_testkit::write_file(
            &root,
            "app/src/Main.elm",
            "module Main exposing (x)\n\n\nx =\n    1\n",
        );
        let snapshot = nomi_engine::Vfs::new().snapshot();
        let found = project_problems(
            &snapshot,
            &[path, main],
            &CheckSettings::default(),
            true,
            &mut Reviewed::default(),
            ColumnUnit::Utf16,
            |_, _, _| Ok((None, None)),
        )
        .unwrap();
        let elm: Vec<&str> = (found.iter())
            .filter(|p| p.code == Code::ElmMake)
            .map(|p| p.message.as_str())
            .collect();
        assert_eq!(elm, [] as [&str; 0]);
    }

    #[test]
    fn elm_analyse_json_turns_rules_off_and_files_out() {
        let dir = tempfile::tempdir().unwrap();
        let json = r#"{ "checks": { "UnusedImport": false, "SingleFieldRecord": true },
                        "excludedPaths": ["src/Generated"] }"#;
        std::fs::write(dir.path().join("elm-analyse.json"), json).unwrap();
        let analyse = Analyse::read(&nomi_engine::Vfs::new().snapshot(), dir.path());
        assert!(!analyse.checks.on(Rule::UnusedImport));
        assert!(analyse.checks.on(Rule::UnusedPatternVariable));
        assert!(analyse.checks.on(Rule::SingleFieldRecord));
        assert!(analyse.excludes(&dir.path().join("src/Generated/Api.elm")));
        assert!(!analyse.excludes(&dir.path().join("src/Main.elm")));
    }

    #[test]
    fn a_plan_indexes_for_annotations_or_ports_from_the_first_linted_file() {
        let analyse = Analyse {
            checks: Checks::default(),
            excluded: vec![PathBuf::from("src/Generated")],
        };
        let open = [
            PathBuf::from("src/Generated/Api.elm"),
            PathBuf::from("src/Main.elm"),
            PathBuf::from("src/Ports.elm"),
        ];
        let read = |p: &Path| p.ends_with("Ports.elm").then(|| "port module".to_owned());
        let settings = CheckSettings::default();
        let plan = Plan::of(&settings, &analyse, &open, read);
        let linted: Vec<&Path> = plan.linted.iter().map(|(p, _)| *p).collect();
        assert_eq!(
            linted,
            [Path::new("src/Main.elm"), Path::new("src/Ports.elm")]
        );
        assert!(plan.annotations);
        assert_eq!(plan.index_from, Some(Path::new("src/Main.elm")));
        // On save only: no annotations, but ports still index.
        let settings = CheckSettings {
            on_save_only: true,
            ..CheckSettings::default()
        };
        let plan = Plan::of(&settings, &analyse, &open, read);
        assert!(!plan.annotations);
        assert_eq!(plan.index_from, Some(Path::new("src/Main.elm")));
        // No lint rules and no ports: no index, and nothing read.
        let settings = CheckSettings {
            no_lint: true,
            ..CheckSettings::default()
        };
        let analyse = Analyse {
            checks: Checks::new([(Rule::MissingTypeAnnotation, false)]),
            ..analyse
        };
        let plan = Plan::of(&settings, &analyse, &open, read);
        assert!(plan.linted.iter().all(|(_, text)| text.is_none()));
        assert_eq!(plan.index_from, None);
    }

    #[test]
    fn review_reruns_on_a_clean_save_and_forgets_on_problems() {
        assert_eq!(Review::after(true, Elm::Clean), Review::Rerun);
        assert_eq!(Review::after(true, Elm::Problems), Review::Forget);
        assert_eq!(Review::after(false, Elm::Problems), Review::Forget);
        assert_eq!(Review::after(false, Elm::Clean), Review::Keep);
    }
}
