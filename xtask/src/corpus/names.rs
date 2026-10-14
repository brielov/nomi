//! `cargo xtask corpus names`: whether every corpus app and package each
//! Compat accepts canonicalizes cleanly, as `nomi check` runs it.
//! Each project's dependencies are checked out and
//! downloaded as `corpus deps` does, through nomi's registry code against
//! the live registry, into the development cache; so this runs by hand,
//! never in CI.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use nomi_base::Compat;
use nomi_engine::check::{self, Build, Built, Checked, Goal};
use nomi_engine::console::Personality;
use nomi_engine::graph::{Env, Foreigns};
use nomi_engine::{Vfs, names};
use nomi_project::{ModuleName, Name, Outline};
use nomi_registry::{Details, Index, Registry, Store, Web, details};

use super::{apps, compat, corpus_dir, packages, parallel};
use crate::oracle::cache_dir;
use crate::oracle::corpus_runs::Projects;

/// Projects that fail as elm fails, each with why, by case.
const EXPECTED: &str = "corpus/names/expected.tsv";

/// A project to check: where it is, canonical, its outline as the Compat
/// reads it, and the paths to build from (none for a package's exposed
/// modules).
pub(super) struct Project {
    pub case: String,
    pub root: PathBuf,
    pub outline: Outline,
    pub paths: Vec<PathBuf>,
}

pub(crate) fn check(root: &Path, only: Option<&str>) -> Result<()> {
    let classification = compat::load(root)?;
    let live = Live::new()?;
    let mut failures: Vec<(String, String)> = Vec::new();
    let mut unavailable: Vec<String> = Vec::new();
    let mut checked = 0;
    for &c in &classification.compats {
        let solving = solve(root, &live, &classification, c, only, Projects::All)?;
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
                let (built, _) = build(c, project, sources, interfaces, Goal::Check)?;
                problems(built.checked)
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
    let expected = Expected::load(root, EXPECTED)?;
    let known = expected.report(&failures, only.is_none());
    println!(
        "names: {} of {checked} projects canonicalize cleanly, {known} fail as elm does, {} unavailable",
        checked - failures.len(),
        unavailable.len()
    );
    if failures.len() > known {
        bail!("{} unexpected failures", failures.len() - known);
    }
    Ok(())
}

/// What `nomi check` finds wrong, if anything, as one line.
fn problems(checked: Checked) -> Result<(), String> {
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

/// `nomi check`'s build of `project` for `goal`, as elm builds it, with
/// `sources` its dependencies' and `interfaces` theirs; and the
/// environment it was built in.
pub(super) fn build(
    compat: Compat,
    project: &Project,
    sources: &[(Name, PathBuf)],
    interfaces: &names::PackageInterfaces,
    goal: Goal,
) -> Result<(Built, Env), String> {
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
        let bytes = fs::read(dir.join("elm.json")).map_err(|e| e.to_string())?;
        let Ok(Outline::Pkg(pkg)) = Outline::decode(&bytes, compat) else {
            return Err(format!("{name}'s elm.json"));
        };
        exposing.push((
            name.clone(),
            pkg.exposed.modules().cloned().collect::<Vec<ModuleName>>(),
        ));
    }
    let visible = names::visible(interfaces, &direct);
    let env = check::env_of(
        &project.root,
        &project.outline,
        compat,
        Foreigns::new(exposing),
    )
    .map_err(|r| r.title.to_string())?
    .with_cwd(&project.root);
    let build = Build {
        snapshot: &Vfs::new().snapshot(),
        env: &env,
        outline: &project.outline,
        paths: &project.paths,
        personality: Personality::Elm,
        packages: (&visible).into(),
        artifacts: None,
    };
    let built = build.run(goal).expect("nothing revises the corpus");
    Ok((built, env))
}

/// The failures a suite expects, each checked by hand: its
/// `expected.tsv`'s cases, each with why.
pub(super) struct Expected {
    path: String,
    cases: BTreeMap<String, String>,
}

impl Expected {
    /// The expected failures `path` lists, from the repository's root;
    /// none if there is no such file.
    pub(super) fn load(root: &Path, path: &str) -> Result<Expected> {
        let text = fs::read_to_string(root.join(path)).unwrap_or_default();
        let mut cases = BTreeMap::new();
        for line in text
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let (case, why) = (line.split_once('\t'))
                .with_context(|| format!("{path}: malformed row `{line}`"))?;
            cases.insert(case.to_owned(), why.to_owned());
        }
        let path = path.to_owned();
        Ok(Expected { path, cases })
    }

    /// The cases listed that aren't among `failures`.
    fn passing<'a>(&'a self, failures: &[(String, String)]) -> Vec<&'a str> {
        (self.cases.keys())
            .filter(|case| !failures.iter().any(|(failed, _)| failed == *case))
            .map(String::as_str)
            .collect()
    }

    /// Prints each of `failures` not listed, and with `all`, each case
    /// listed that passed, to be removed; gives how many are listed.
    pub(super) fn report(&self, failures: &[(String, String)], all: bool) -> usize {
        let mut known = 0;
        for (case, why) in failures {
            if self.cases.contains_key(case) {
                known += 1;
            } else {
                eprintln!("FAIL {case}: {why}");
            }
        }
        if all {
            for case in self.passing(failures) {
                eprintln!("now passes, remove it from {}: {case}", self.path);
            }
        }
        known
    }
}

/// The live registry, through the development cache's own nomi cache,
/// never the user's.
pub(super) struct Live {
    cache: PathBuf,
    store: Store,
    http: Web,
}

impl Live {
    pub(super) fn new() -> Result<Live> {
        let cache = cache_dir()?.join("nomi");
        let store = Store::new(cache.join("packages"));
        Ok(Live {
            cache,
            store,
            http: Web::new(),
        })
    }

    /// The registry as `c` reads it, its index brought up to date.
    pub(super) fn registry(&self, c: Compat) -> Result<Registry<'_, Web>> {
        let (index, connection) = Index::latest(&self.http, &self.cache.join("registry/index"), c)
            .map_err(|e| anyhow!("loading the index: {e:?}"))?;
        Ok(Registry::new(
            index,
            connection,
            &self.store,
            self.cache.join("registry/metadata"),
            &self.http,
            c,
        ))
    }

    /// Each release of `solution`, downloaded into the store, or why it
    /// wasn't.
    pub(super) fn download(
        &self,
        solution: &BTreeMap<Name, Details>,
        c: Compat,
    ) -> Vec<Result<(Name, PathBuf), String>> {
        let releases: Vec<_> = (solution.iter())
            .map(|(name, details)| (name, details.version))
            .collect();
        parallel(&releases, |(name, version)| {
            (self.store.fetch(&self.http, name, *version, c))
                .map(|dir| ((*name).clone(), dir))
                .map_err(|e| format!("{name} {version}: {e:?}"))
        })
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

/// The corpus `projects` `c` accepts whose case contains `only`, each with
/// its dependencies solved and downloaded into the development cache.
pub(super) fn solve(
    root: &Path,
    live: &Live,
    classification: &compat::Classification,
    c: Compat,
    only: Option<&str>,
    projects: Projects,
) -> Result<Solving> {
    let corpus = corpus_dir()?;
    let mut failures = Vec::new();
    let mut unavailable = Vec::new();
    let mut found = Vec::new();
    let apps = if projects.apps() {
        apps::load(root)?.app
    } else {
        Vec::new()
    };
    for app in apps {
        let text = compat::sandbox_manifest(classification, &app, c)?;
        let outline =
            Outline::read(text.as_bytes(), c).map_err(|e| anyhow!("{}: {e:?}", app.name))?;
        found.push(Project {
            case: format!("app/{} on {c}", app.name),
            root: fs::canonicalize(apps::app_dir(&corpus, &app).join(&app.root))?,
            outline,
            paths: app.entries.iter().map(PathBuf::from).collect(),
        });
    }
    let packages = if projects.packages() {
        packages::load(root)?.packages
    } else {
        Vec::new()
    };
    for package in packages {
        if !classification.admits(&package.name, &package.version, c) {
            continue;
        }
        let dir = packages::package_dir(&corpus, &package);
        let Ok(bytes) = fs::read(dir.join("elm.json")) else {
            continue;
        };
        let Ok(outline) = Outline::read(&bytes, c) else {
            continue;
        };
        found.push(Project {
            case: format!("{}@{} on {c}", package.name, package.version),
            root: fs::canonicalize(dir)?,
            outline,
            paths: Vec::new(),
        });
    }
    found.retain(|p| only.is_none_or(|only| p.case.contains(only)));
    let mut registry = live.registry(c)?;
    // Each project's releases, solved and downloaded.
    let mut solved = Vec::new();
    for project in found {
        let solution = match details::verify(&mut registry, c, &project.outline) {
            Ok(solution) => solution,
            Err(e) => {
                failures.push((project.case.clone(), format!("dependencies: {e:?}")));
                continue;
            }
        };
        let downloads = live.download(&solution, c);
        // A release the registry no longer serves as it published it
        // stops elm as it stops nomi: nothing to compare.
        if let Some(why) = downloads
            .iter()
            .filter_map(|d| d.as_ref().err())
            .next_back()
        {
            unavailable.push(format!("{}: {why}", project.case));
            continue;
        }
        solved.push((project, downloads.into_iter().flatten().collect()));
    }
    Ok(Solving {
        solved,
        failures,
        unavailable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listed_cases_that_pass_are_found() {
        let expected = Expected {
            path: EXPECTED.to_owned(),
            cases: [("a on 0.19.1", "why"), ("b on 0.19.1", "why")]
                .map(|(case, why)| (case.to_owned(), why.to_owned()))
                .into(),
        };
        let failures = [("a on 0.19.1".to_owned(), "it fails".to_owned())];
        assert_eq!(expected.passing(&failures), ["b on 0.19.1"]);
    }
}
