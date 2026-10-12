//! `cargo xtask corpus names`: whether every corpus app and package each
//! Compat accepts (1.15a) canonicalizes cleanly, as `nomi check` runs it
//! (plan 5.13). Each project's dependencies are checked out and
//! downloaded as `corpus deps` does, through nomi's registry code against
//! the live registry, into the development cache; so this runs by hand,
//! never in CI.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use nomi_base::Compat;
use nomi_compiler::interface::Interface;
use nomi_engine::check::{self, Checked};
use nomi_engine::console::Personality;
use nomi_engine::graph::Foreigns;
use nomi_engine::{Vfs, names};
use nomi_project::{ModuleName, Name, Outline};
use nomi_registry::{Index, Registry, Store, Web, details};

use super::{apps, compat, corpus_dir, packages, parallel};
use crate::oracle::cache_dir;

/// A project to check: where it is, its outline as the Compat reads it,
/// and the paths to build from (none for a package's exposed modules).
/// Projects that fail as elm fails, each with why, by case.
const EXPECTED: &str = "corpus/names/expected.tsv";

pub(super) struct Project {
    pub case: String,
    pub root: PathBuf,
    pub outline: Outline,
    pub paths: Vec<PathBuf>,
}

pub fn check(root: &Path, only: Option<&str>) -> Result<()> {
    let classification = compat::load(root)?;
    let mut failures: Vec<(String, String)> = Vec::new();
    let mut unavailable: Vec<String> = Vec::new();
    let mut checked = 0;
    for v in &classification.compats {
        let c = (Compat::ALL.into_iter())
            .find(|c| c.to_string() == *v)
            .with_context(|| format!("nomi doesn't emulate elm {v}"))?;
        let solving = solve(root, &classification, c, v, only, true, true)?;
        checked += solving.failures.len();
        failures.extend(solving.failures);
        unavailable.extend(solving.unavailable);
        let solved = solving.solved;
        // A chunk at a time: interfaces made, projects checked, then
        // dropped, so only the packages' own interfaces stay in memory.
        let mut memo = HashMap::new();
        for chunk in solved.chunks(64) {
            let mut ready = Vec::new();
            for (project, sources) in chunk {
                // A package builds as elm builds a dependency: from its
                // exposed modules, its kernel imports unchecked.
                let mut sources = sources.clone();
                if let Outline::Pkg(pkg) = &project.outline {
                    sources.push((pkg.name.clone(), project.root.clone()));
                }
                match names::package_interfaces_with(c, &sources, &mut memo) {
                    Ok(_) if matches!(project.outline, Outline::Pkg(_)) => checked += 1,
                    Ok(interfaces) => ready.push((project, sources, interfaces)),
                    Err(why) => {
                        checked += 1;
                        let why: String = why.chars().take(300).collect();
                        failures.push((project.case.clone(), why));
                    }
                }
            }
            let results = parallel(&ready, |(project, sources, interfaces)| {
                run(c, project, sources, interfaces)
            });
            for ((project, _, _), result) in ready.iter().zip(results) {
                checked += 1;
                if let Err(why) = result {
                    failures.push((project.case.clone(), why));
                }
            }
            eprintln!("  {c}: {checked} checked, {} failing", failures.len());
        }
    }
    for why in &unavailable {
        eprintln!("unavailable: {why}");
    }
    // Packages elm rejects too, each checked by hand, with why.
    let expected_path = root.join(EXPECTED);
    let expected: BTreeMap<String, String> = std::fs::read_to_string(&expected_path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| l.split_once('\t'))
        .map(|(case, why)| (case.to_owned(), why.to_owned()))
        .collect();
    let failing: BTreeMap<&str, &str> = failures
        .iter()
        .map(|(c, w)| (c.as_str(), w.as_str()))
        .collect();
    let unexpected: Vec<_> = failing
        .iter()
        .filter(|(c, _)| !expected.contains_key(**c))
        .collect();
    for (case, why) in &unexpected {
        eprintln!("FAIL {case}: {why}");
    }
    for case in expected
        .keys()
        .filter(|c| !failing.contains_key(c.as_str()))
    {
        if only.is_none() {
            eprintln!("now passes, remove it from {EXPECTED}: {case}");
        }
    }
    println!(
        "names: {} of {checked} projects canonicalize cleanly, {} fail as elm does, {} unavailable",
        checked - failures.len(),
        failures.len() - unexpected.len(),
        unavailable.len()
    );
    if !unexpected.is_empty() {
        bail!("{} unexpected failures", unexpected.len());
    }
    Ok(())
}

/// `nomi check` on `project`, which must find nothing wrong.
fn run(
    compat: Compat,
    project: &Project,
    sources: &[(Name, PathBuf)],
    interfaces: &names::PackageInterfaces,
) -> Result<(), String> {
    let direct: Vec<Name> = match &project.outline {
        Outline::App(app) => app.direct.keys().cloned().collect(),
        Outline::Pkg(pkg) => pkg.deps.keys().cloned().collect(),
    };
    let dirs: BTreeMap<&Name, &PathBuf> = sources.iter().map(|(n, d)| (n, d)).collect();
    let mut exposing = Vec::new();
    for name in &direct {
        let dir = dirs
            .get(name)
            .ok_or_else(|| format!("{name} wasn't downloaded"))?;
        let bytes = std::fs::read(dir.join("elm.json")).map_err(|e| e.to_string())?;
        let Ok(Outline::Pkg(pkg)) = Outline::decode(&bytes, compat) else {
            return Err(format!("{name}'s elm.json"));
        };
        exposing.push((
            name.clone(),
            pkg.exposed.modules().cloned().collect::<Vec<ModuleName>>(),
        ));
    }
    let visible: HashMap<String, Interface> = names::visible(interfaces, &direct);
    let env = check::env_of(
        &project.root,
        &project.outline,
        compat,
        Foreigns::new(exposing),
    )
    .map_err(|r| r.title.to_string())?
    .with_cwd(&project.root);
    let checked = check::check(
        &Vfs::new().snapshot(),
        &env,
        &project.outline,
        &project.paths,
        Personality::Elm,
        (&visible).into(),
    );
    match checked {
        Checked::Fine => Ok(()),
        Checked::Project(report) => Err(report.title.to_string()),
        Checked::Modules(modules) => Err(modules
            .iter()
            .map(|m| {
                let problems: Vec<String> = (m.problems.iter())
                    .map(|p| format!("{}:{} {}", p.region.start.row, p.region.start.col, p.title))
                    .collect();
                format!("{} {}", m.path.display(), problems.join(", "))
            })
            .collect::<Vec<_>>()
            .join("; ")),
    }
}

/// Corpus projects, each with its dependencies' sources.
pub(super) struct Solving {
    pub solved: Vec<(Project, Vec<(Name, PathBuf)>)>,
    /// Projects whose dependencies have no solution, with why.
    pub failures: Vec<(String, String)>,
    /// Projects a release the registry no longer serves stops.
    pub unavailable: Vec<String>,
}

/// The corpus projects `c` (the elm release `v`) accepts whose case
/// contains `only`, apps if `apps` and packages if `packages`, each with
/// its dependencies solved and downloaded into the development cache.
pub(super) fn solve(
    root: &Path,
    classification: &compat::Classification,
    c: Compat,
    v: &str,
    only: Option<&str>,
    apps: bool,
    packages: bool,
) -> Result<Solving> {
    let corpus = corpus_dir()?;
    let cache = cache_dir()?.join("nomi");
    let store = Store::new(cache.join("packages"));
    let http = Web::new();
    let mut failures = Vec::new();
    let mut unavailable = Vec::new();
    let mut projects = Vec::new();
    let apps = if apps {
        apps::load(root)?.app
    } else {
        Vec::new()
    };
    for app in apps {
        let text = compat::sandbox_manifest(classification, &app, compat::Version::parse(v)?)?;
        let outline =
            Outline::read(text.as_bytes(), c).map_err(|e| anyhow!("{}: {e:?}", app.name))?;
        projects.push(Project {
            case: format!("app/{} on {c}", app.name),
            root: apps::app_dir(&corpus, &app).join(&app.root),
            outline,
            paths: app.entries.iter().map(PathBuf::from).collect(),
        });
    }
    let packages = if packages {
        packages::load(root)?.packages
    } else {
        Vec::new()
    };
    for package in packages {
        let excluded = (classification.package.iter())
            .find(|p| p.name == package.name && p.version == package.version)
            .is_some_and(|p| !p.admits.iter().any(|a| a == v));
        if excluded {
            continue;
        }
        let dir = packages::package_dir(&corpus, &package);
        let Ok(bytes) = std::fs::read(dir.join("elm.json")) else {
            continue;
        };
        let Ok(outline) = Outline::read(&bytes, c) else {
            continue;
        };
        projects.push(Project {
            case: format!("{}@{} on {c}", package.name, package.version),
            root: dir,
            outline,
            paths: Vec::new(),
        });
    }
    projects.retain(|p| only.is_none_or(|only| p.case.contains(only)));
    let (index, connection) = Index::latest(&http, &cache.join("registry").join("index"), c)
        .map_err(|e| anyhow!("loading the index: {e:?}"))?;
    let mut registry = Registry::new(
        index,
        connection,
        &store,
        cache.join("registry").join("metadata"),
        &http,
        c,
    );
    // Each project's releases, solved and downloaded.
    let mut solved = Vec::new();
    for project in projects {
        let solution = match details::verify(&mut registry, c, &project.outline) {
            Ok(solution) => solution,
            Err(e) => {
                failures.push((project.case.clone(), format!("dependencies: {e:?}")));
                continue;
            }
        };
        let releases: Vec<_> = solution
            .iter()
            .map(|(n, d)| (n.clone(), d.version))
            .collect();
        let dirs = parallel(&releases, |(name, version)| {
            store.fetch(&http, name, *version, c)
        });
        let mut sources = Vec::new();
        let mut missing = None;
        for ((name, version), dir) in releases.iter().zip(dirs) {
            match dir {
                Ok(dir) => sources.push((name.clone(), dir)),
                Err(e) => missing = Some(format!("{name} {version}: {e:?}")),
            }
        }
        // A release the registry no longer serves as it published it
        // stops elm as it stops nomi: nothing to compare.
        if let Some(why) = missing {
            unavailable.push(format!("{}: {why}", project.case));
            continue;
        }
        solved.push((project, sources));
    }
    Ok(Solving {
        solved,
        failures,
        unavailable,
    })
}
