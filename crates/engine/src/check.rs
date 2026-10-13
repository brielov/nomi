//! `nomi check`: what `elm make --output=/dev/null` reports, as far as
//! nomi's phases go (`Make.hs`, `Build.hs`). The project and its
//! dependencies load first, then the modules the paths name, or a
//! package's exposed ones, are crawled. Only the earliest failing phase of
//! each module is shown, and a module whose imports failed stays silent;
//! so far the phases are syntax, module names and imports.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nomi_base::diagnostic::doc::{Color, reflow, text, vcat};
use nomi_base::diagnostic::json;
use nomi_base::{Compat, Report};
use nomi_compiler::interface::Interface;
use nomi_compiler::optimized::LocalGraph;
use nomi_project::{ModuleName, Name, Outline, Version};
use nomi_registry::{Http, Store, details, report as registry_report};

use crate::artifacts::{Artifacts, Key};
use crate::console::{self, Console, Personality};
use crate::graph::{Env, Foreigns, Graph, Owner};
use crate::install::{details_problem, one_line};
use crate::make::{self, Make};
use crate::names::{Goals, Packages};
use crate::paths::Cache;
use crate::project::Project;
use crate::report::{self, ModuleErrors};
use crate::{Pool, Snapshot, Vfs, env, names, objects, packages};

/// What checking found.
#[derive(Debug)]
pub enum Checked {
    /// Nothing nomi checks is wrong.
    Fine,
    /// The project can't be built at all.
    Project(Report),
    /// These modules have problems, in upstream's order.
    Modules(Vec<ModuleErrors>),
}

/// How the outcome is shown: `--report=json` or for people.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Terminal,
    Json,
}

/// Checks the project around `dir`, building from `paths`, or a
/// package's exposed modules when there are none, and shows what it
/// found: `elm make --output=/dev/null`, as [`make::run`] runs it.
/// Whether nothing was wrong.
pub fn run(
    vfs: &Vfs,
    console: &mut dyn Console,
    dir: &Path,
    cache: &Cache,
    http: &(impl Http + Sync),
    paths: &[PathBuf],
    style: Style,
) -> bool {
    make::run(vfs, console, dir, cache, http, &Make::check(paths, style))
}

/// Checks the modules of the project `env` describes, as `elm make`
/// builds from `paths`, or from the package's exposed modules when there
/// are none.
pub fn check(
    snapshot: &Snapshot,
    env: &Env,
    outline: &Outline,
    paths: &[PathBuf],
    personality: Personality,
    packages: Packages<'_>,
) -> Checked {
    check_with(snapshot, env, outline, paths, personality, packages, None).0
}

/// [`check`], reusing what `artifacts` holds of earlier checks; and the
/// modules it checked rather than reused.
pub fn check_with(
    snapshot: &Snapshot,
    env: &Env,
    outline: &Outline,
    paths: &[PathBuf],
    personality: Personality,
    packages: Packages<'_>,
    artifacts: Option<&Artifacts>,
) -> (Checked, Vec<PathBuf>) {
    let built = build(
        snapshot,
        env,
        outline,
        paths,
        personality,
        packages,
        artifacts,
        Goal::Check,
    );
    (built.checked, built.ran)
}

/// [`check_with`] for `elm make`: also each root and whether it defines
/// `main`, and how many modules came out clean.
pub fn make_with(
    snapshot: &Snapshot,
    env: &Env,
    outline: &Outline,
    paths: &[PathBuf],
    personality: Personality,
    packages: Packages<'_>,
    artifacts: Option<&Artifacts>,
) -> (Checked, Vec<(ModuleName, bool)>, usize) {
    let built = build(
        snapshot,
        env,
        outline,
        paths,
        personality,
        packages,
        artifacts,
        Goal::Check,
    );
    (built.checked, built.mains, built.compiled)
}

/// What `elm make --docs` makes of the package `env` describes: its
/// exposed modules checked as [`check`] checks them and documented, and
/// `docs.json` if nothing was wrong, and how many modules came out clean.
pub fn docs(
    snapshot: &Snapshot,
    env: &Env,
    outline: &Outline,
    personality: Personality,
    packages: Packages<'_>,
) -> (Checked, Option<String>, usize) {
    docs_with(snapshot, env, outline, personality, packages, None)
}

/// [`docs`], reusing what `artifacts` holds of the modules it doesn't
/// document.
pub fn docs_with(
    snapshot: &Snapshot,
    env: &Env,
    outline: &Outline,
    personality: Personality,
    packages: Packages<'_>,
    artifacts: Option<&Artifacts>,
) -> (Checked, Option<String>, usize) {
    let built = build(
        snapshot,
        env,
        outline,
        &[],
        personality,
        packages,
        artifacts,
        Goal::Docs,
    );
    let json = matches!(built.checked, Checked::Fine)
        .then(|| nomi_compiler::docs::encode(env.compat(), &built.docs));
    (built.checked, json, built.compiled)
}

/// What [`build`] makes beyond the problems.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Goal {
    Check,
    /// A package's exposed modules' documentation.
    Docs,
    /// Every module's optimized graph.
    Optimize,
}

/// What [`build`] found and made.
struct Built {
    checked: Checked,
    /// The modules checked rather than reused.
    ran: Vec<PathBuf>,
    docs: Vec<nomi_compiler::docs::Module>,
    /// The roots, and whether each defines `main`.
    mains: Vec<(ModuleName, bool)>,
    optimized: Vec<(ModuleName, LocalGraph)>,
    interfaces: Vec<(ModuleName, Arc<names::LazyInterface>)>,
    /// How many modules came out clean.
    compiled: usize,
}

impl Built {
    fn failed(checked: Checked) -> Built {
        Built {
            checked,
            ran: Vec::new(),
            docs: Vec::new(),
            mains: Vec::new(),
            optimized: Vec::new(),
            interfaces: Vec::new(),
            compiled: 0,
        }
    }
}

/// What [`optimize`] makes of a project.
#[derive(Clone, Debug, Default)]
pub struct Optimized {
    /// The roots, and whether each defines `main`.
    pub roots: Vec<(ModuleName, bool)>,
    /// Each module's graph.
    pub graphs: Vec<(ModuleName, LocalGraph)>,
    /// Each module's interface.
    pub interfaces: Vec<(ModuleName, Arc<names::LazyInterface>)>,
    /// The modules checked rather than taken from the artifact cache.
    pub ran: Vec<PathBuf>,
    /// How many modules came out clean, checked or reused.
    pub compiled: usize,
}

/// What `elm make` optimizes of the project `env` describes, building
/// as [`check`] does, when nothing was wrong.
pub fn optimize(
    snapshot: &Snapshot,
    env: &Env,
    outline: &Outline,
    paths: &[PathBuf],
    personality: Personality,
    packages: Packages<'_>,
    artifacts: Option<&Artifacts>,
) -> (Checked, Optimized) {
    let built = build(
        snapshot,
        env,
        outline,
        paths,
        personality,
        packages,
        artifacts,
        Goal::Optimize,
    );
    let optimized = Optimized {
        roots: built.mains,
        graphs: built.optimized,
        interfaces: built.interfaces,
        ran: built.ran,
        compiled: built.compiled,
    };
    (built.checked, optimized)
}

/// [`check_with`], making what `goal` asks for too.
#[expect(
    clippy::too_many_arguments,
    reason = "each of the checks above passes its own mix of these"
)]
fn build(
    snapshot: &Snapshot,
    env: &Env,
    outline: &Outline,
    paths: &[PathBuf],
    personality: Personality,
    packages: Packages<'_>,
    artifacts: Option<&Artifacts>,
    goal: Goal,
) -> Built {
    let exposed: Vec<ModuleName> = match outline {
        Outline::Pkg(pkg) => pkg.exposed.modules().cloned().collect(),
        Outline::App(_) => Vec::new(),
    };
    let graph = if !paths.is_empty() {
        Graph::from_paths(snapshot, env, paths)
    } else if let Outline::App(_) = outline {
        return Built::failed(Checked::Project(no_input(personality, true)));
    } else if exposed.is_empty() {
        return Built::failed(Checked::Project(no_input(personality, false)));
    } else {
        Graph::from_exposed(snapshot, env, &exposed)
    };
    let graph = match graph {
        Ok(graph) => graph,
        Err(problem) => {
            return Built::failed(Checked::Project(report::project_problem(
                &problem,
                env.compat(),
            )));
        }
    };
    if paths.is_empty()
        && let Some(report) = report::missing_exposed(&graph, &exposed)
    {
        return Built::failed(Checked::Project(report));
    }
    let earlier = report::module_errors(env, &graph);
    let failed: HashSet<PathBuf> = earlier.iter().map(|m| m.path.clone()).collect();
    let documented: &[ModuleName] = if goal == Goal::Docs && paths.is_empty() {
        &exposed
    } else {
        &[]
    };
    let goals = Goals {
        documented,
        optimize: goal == Goal::Optimize,
    };
    let later = names::check_graph(env, &graph, packages, &failed, artifacts, goals);
    let modules = report::module_errors_with(env, &graph, &later.problems);
    let checked = if modules.is_empty() {
        Checked::Fine
    } else {
        Checked::Modules(modules)
    };
    Built {
        checked,
        ran: later.ran,
        docs: later.docs,
        mains: graph.root_mains(),
        optimized: later.optimized,
        interfaces: later.interfaces,
        compiled: later.compiled,
    }
}

/// Shows `checked`: in the terminal style, the reports on standard error
/// and a line saying how it went on standard output (`toFinalMessage`);
/// in JSON, the report alone, on standard error, as `encodeUgly` writes it.
pub fn show(
    console: &mut dyn Console,
    root: &Path,
    checked: &Checked,
    compat: Compat,
    style: Style,
) {
    match (checked, style) {
        (Checked::Fine, Style::Terminal) => {
            console.out("No problems in syntax, module names, imports, names or types.\n");
        }
        (Checked::Fine, Style::Json) => {}
        (Checked::Project(report), Style::Terminal) => console::report(console, report, compat),
        (Checked::Project(report), Style::Json) => {
            console.err(&json::report(report, compat).to_string());
        }
        (Checked::Modules(modules), Style::Terminal) => {
            let summary = match modules.len() {
                1 => "Detected problems in 1 module.".to_owned(),
                n => format!("Detected problems in {n} modules."),
            };
            console.out(&format!("{summary}\n"));
            console::report_doc(console, &report::modules_doc(root, modules), compat);
        }
        (Checked::Modules(modules), Style::Json) => {
            let modules = modules.iter().map(|m| m.to_json(compat)).collect();
            console.err(&json::compile_errors(modules).to_string());
        }
    }
}

/// `MakeAppNeedsFileNames` and `MakePkgNeedsExposing`.
fn no_input(personality: Personality, app: bool) -> Report {
    let command = personality.command(match personality {
        Personality::Nomi => "check",
        Personality::Elm => "make",
    });
    let examples = vcat([
        text(format!("{command} src/Main.elm"))
            .color(Color::Green)
            .indent(4),
        text(format!("{command} src/This.elm src/That.elm"))
            .color(Color::Green)
            .indent(4),
    ]);
    let advice = if app {
        "I recommend reading through https://guide.elm-lang.org for guidance on what to \
         actually put in those files!"
    } else {
        "You can also entries to the \"exposed-modules\" list in your elm.json file, and I \
         will try to compile the relevant files."
    };
    Report::new(
        "NO INPUT",
        None,
        "What should I make though? I need specific files like:",
        [examples, reflow(advice)],
    )
}

/// What a project builds with, as `Details.load` finds it.
pub(crate) struct Sources {
    pub foreigns: Foreigns,
    /// Each direct dependency, in order of names, and the modules it
    /// exposes.
    pub direct: Vec<(Name, Vec<ModuleName>)>,
    /// Every package, and the directory of its source.
    pub packages: Vec<(Name, PathBuf)>,
}

/// What the project builds with, every release downloaded first.
pub(crate) fn sources(
    cache: &Cache,
    http: &(impl Http + Sync),
    project: &Project,
    personality: Personality,
) -> Result<Sources, Report> {
    let compat = project.compat;
    let store = Store::new(cache.packages());
    // elm keeps a project's dependencies as solved until its elm.json
    // changes (`Details.load`), asking the website nothing; nomi keeps
    // them by what elm.json says.
    let artifacts = Artifacts::new(cache);
    let key = Key::new("details")
        .part("compat", compat.to_string().as_bytes())
        .part("outline", &project.outline.write())
        .finish();
    let kept = artifacts
        .get(key)
        .and_then(|bytes| decode_releases(&bytes, compat));
    let releases = if let Some(releases) = kept {
        releases
    } else {
        let mut registry = env::registry(
            cache,
            &store,
            http,
            compat,
            "I need the list of published packages to check your dependencies",
        )?;
        let solution = details::verify(&mut registry, compat, &project.outline)
            .map_err(|problem| details_problem(&problem, compat, personality))?;
        let releases: Vec<(Name, Version)> = solution
            .iter()
            .map(|(name, details)| (name.clone(), details.version))
            .collect();
        // A cache that can't be written only costs a solve next time.
        let _ = artifacts.put(key, encode_releases(&releases).as_bytes());
        releases
    };
    let fetched = packages::fetch(Pool::global(), &store, http, compat, releases.clone());
    let direct: Vec<&Name> = match &project.outline {
        Outline::App(app) => app.direct.keys().collect(),
        Outline::Pkg(pkg) => pkg.deps.keys().collect(),
    };
    let mut exposing = HashMap::new();
    let mut packages = Vec::new();
    for ((name, version), dir) in releases.iter().zip(fetched) {
        let dir =
            dir.map_err(|problem| registry_report::package_problem(name, *version, &problem))?;
        packages.push((name.clone(), dir.clone()));
        if !direct.contains(&name) {
            continue;
        }
        let unreadable = || {
            one_line(
                "CORRUPT PACKAGE DATA",
                &format!(
                    "I could not read the elm.json of {name} {version} in {}.",
                    dir.display()
                ),
            )
        };
        let bytes = std::fs::read(dir.join("elm.json")).map_err(|_| unreadable())?;
        match Outline::decode(&bytes, compat) {
            Ok(Outline::Pkg(pkg)) => {
                exposing.insert(name.clone(), pkg.exposed.modules().cloned().collect());
            }
            _ => return Err(unreadable()),
        }
    }
    let direct: Vec<(Name, Vec<ModuleName>)> = (direct.into_iter())
        .filter_map(|name| Some((name.clone(), exposing.remove(name)?)))
        .collect();
    Ok(Sources {
        foreigns: Foreigns::new(direct.iter().cloned()),
        direct,
        packages,
    })
}

/// `releases`, one `author/project@version` a line.
fn encode_releases(releases: &[(Name, Version)]) -> String {
    (releases.iter())
        .map(|(name, version)| format!("{name}@{version}\n"))
        .collect()
}

fn decode_releases(bytes: &[u8], compat: Compat) -> Option<Vec<(Name, Version)>> {
    (bytes.split(|&b| b == b'\n'))
        .filter(|line| !line.is_empty())
        .map(|line| {
            Name::parse_release(line, nomi_project::json::Cursor::start(compat), compat).ok()
        })
        .collect()
}

/// The modules each direct dependency exposes, and their interfaces, with
/// every release the project builds with downloaded first, as
/// `Details.load` builds them.
pub(crate) fn foreigns(
    cache: &Cache,
    http: &(impl Http + Sync),
    project: &Project,
    personality: Personality,
) -> Result<(Foreigns, HashMap<String, Interface>, objects::Dependencies), Report> {
    let sources = sources(cache, http, project, personality)?;
    let artifacts = Artifacts::new(cache);
    let built = objects::build(
        project.compat,
        &sources.packages,
        &mut objects::Memo::new(),
        Some(&artifacts),
    );
    let built = built.map_err(|why| packages_problem(&why))?;
    let direct: Vec<Name> = sources.direct.into_iter().map(|(name, _)| name).collect();
    Ok((
        sources.foreigns,
        names::visible(&built.exposed(), &direct),
        built,
    ))
}

/// A package nomi couldn't build.
pub(crate) fn packages_problem(why: &str) -> Report {
    one_line(
        "PROBLEM READING PACKAGES",
        &format!("I could not read the dependencies' modules: {why}"),
    )
}

/// The build environment of the project at `root`.
pub fn env_of(
    root: &Path,
    outline: &Outline,
    compat: Compat,
    foreigns: Foreigns,
) -> Result<Env, Report> {
    let (owner, src_dirs) = match outline {
        Outline::App(app) => (
            Owner::Application,
            app.source_dirs
                .iter()
                .filter_map(|d| d.path())
                .map(PathBuf::from)
                .collect::<Vec<_>>(),
        ),
        Outline::Pkg(pkg) => (Owner::Package(pkg.name.clone()), vec![PathBuf::from("src")]),
    };
    Env::new(compat, root, owner, &src_dirs, foreigns).map_err(|e| {
        one_line(
            "PROBLEM READING PROJECT",
            &format!("I could not read the project at {}: {e}", root.display()),
        )
    })
}
