//! `nomi check`: what `elm make --output=/dev/null` reports, as far as
//! nomi's phases go (`Make.hs`, `Build.hs`). The project and its
//! dependencies load first, then the modules the paths name, or a
//! package's exposed ones, are crawled. Only the earliest failing phase of
//! each module is shown, and a module whose imports failed stays silent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use nomi_base::diagnostic::doc::{Color, reflow, text, vcat};
use nomi_base::diagnostic::json;
use nomi_base::{Compat, Report};
use nomi_compiler::interface::Interface;
use nomi_compiler::optimized::LocalGraph;
use nomi_project::report::ElmCommand;
use nomi_project::{ModuleName, Name, Outline, Version};
use nomi_registry::{Http, Store, details, report as registry_report};

use crate::artifacts::{Artifacts, Key};
use crate::console::{self, Console, Personality};
use crate::graph::{Env, Foreigns, Graph, Owner, RootMain};
use crate::install::details_problem;
use crate::make::{self, Make};
use crate::names::{ModuleInterface, Packages};
use crate::paths::Cache;
use crate::project::{self, Project};
use crate::report::{self, ModuleErrors, one_line};
use crate::schedule::Cancelled;
use crate::{Context, Pool, Snapshot, env, names, objects, packages};

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
    /// Reports for people, with a line saying how it went.
    Terminal,
    /// `--report=json`.
    Json,
}

/// Checks the project around the directory, building from `paths`, or a
/// package's exposed modules when there are none, and shows what it
/// found: `elm make --output=/dev/null`, as [`make::run`] runs it.
/// Whether nothing was wrong.
pub fn run(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    paths: Vec<PathBuf>,
    style: Style,
) -> bool {
    make::run(cx, console, &Make::check(paths, style))
}

/// What to build: the modules of the project `env` describes, as `elm
/// make` builds from `paths`, or from the package's exposed modules when
/// there are none, reading them from `snapshot`.
#[derive(Clone, Copy, Debug)]
pub struct Build<'a> {
    pub snapshot: &'a Snapshot,
    pub env: &'a Env,
    pub outline: &'a Outline,
    pub paths: &'a [PathBuf],
    /// Whose words the reports are in.
    pub personality: Personality,
    pub packages: Packages<'a>,
    /// What earlier builds left, reused (architecture §6).
    pub artifacts: Option<&'a Artifacts>,
}

/// What a build makes beyond the problems.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Goal {
    Check,
    /// A package's exposed modules' documentation, `docs.json`, as `elm
    /// make --docs` makes it.
    Docs,
    /// Every module's optimized graph, as `elm make` makes them before it
    /// generates code.
    Optimize,
}

/// What a build found and made.
#[derive(Debug)]
pub struct Built {
    pub checked: Checked,
    /// The modules checked rather than taken from the artifact cache.
    pub ran: Vec<PathBuf>,
    /// How many modules came out clean, checked or reused.
    pub compiled: usize,
    /// `docs.json`, documenting, if nothing was wrong.
    pub docs: Option<String>,
    /// What optimizing made.
    pub optimized: Optimized,
}

/// What optimizing a project makes.
#[derive(Clone, Debug, Default)]
pub struct Optimized {
    /// The roots, and whether each defines `main`.
    pub roots: Vec<RootMain>,
    /// Each module's graph.
    pub graphs: Vec<(ModuleName, LocalGraph)>,
    /// Each module's interface.
    pub interfaces: Vec<ModuleInterface>,
}

impl Build<'_> {
    /// Builds what `goal` asks for, as `elm make` does; cancelled once the
    /// snapshot is stale.
    pub fn run(&self, goal: Goal) -> Result<Built, Cancelled> {
        let failed = |report| Built {
            checked: Checked::Project(report),
            ran: Vec::new(),
            compiled: 0,
            docs: None,
            optimized: Optimized::default(),
        };
        let (env, paths) = (self.env, self.paths);
        let exposed: Vec<ModuleName> = match self.outline {
            Outline::Pkg(pkg) => pkg.exposed.modules().cloned().collect(),
            Outline::App(_) => Vec::new(),
        };
        let graph = if !paths.is_empty() {
            Graph::from_paths(self.snapshot, env, paths)
        } else if exposed.is_empty() {
            return Ok(failed(no_input(self.personality, self.outline)));
        } else {
            Graph::from_exposed(self.snapshot, env, &exposed)
        };
        let graph = match graph {
            Ok(graph) => graph,
            Err(problem) => return Ok(failed(report::project_problem(&problem, env.compat()))),
        };
        if paths.is_empty()
            && let Some(report) = report::missing_exposed(&graph, &exposed)
        {
            return Ok(failed(report));
        }
        let walk_goal = match goal {
            Goal::Docs if paths.is_empty() => names::Goal::Document(&exposed),
            Goal::Check | Goal::Docs => names::Goal::Check,
            Goal::Optimize => names::Goal::Optimize,
        };
        let walked = names::check_graph(
            env,
            &graph,
            self.packages,
            self.snapshot,
            self.artifacts,
            walk_goal,
        )?;
        let modules = report::module_errors(env, &graph, walked.problems);
        let checked = if modules.is_empty() {
            Checked::Fine
        } else {
            Checked::Modules(modules)
        };
        let docs = (goal == Goal::Docs && matches!(checked, Checked::Fine))
            .then(|| nomi_compiler::docs::encode(env.compat(), &walked.docs));
        let roots = match goal {
            Goal::Optimize => graph.root_mains(),
            Goal::Check | Goal::Docs => Vec::new(),
        };
        Ok(Built {
            checked,
            ran: walked.ran,
            compiled: walked.compiled,
            docs,
            optimized: Optimized {
                roots,
                graphs: walked.optimized,
                interfaces: walked.interfaces,
            },
        })
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
        (Checked::Project(_), Style::Terminal) => show_reports(console, root, checked, compat),
        (Checked::Modules(modules), Style::Terminal) => {
            console.out(&format!("{}\n", problems_summary(modules.len())));
            show_reports(console, root, checked, compat);
        }
        (Checked::Fine, Style::Json) => {}
        (Checked::Project(report), Style::Json) => {
            console.err(&json::report(report, compat).to_string());
        }
        (Checked::Modules(modules), Style::Json) => {
            let modules = modules.iter().map(|m| m.to_json(compat)).collect();
            console.err(&json::compile_errors(modules).to_string());
        }
    }
}

/// The reports of `checked` on standard error, in the terminal style,
/// without the line saying how it went.
pub(crate) fn show_reports(
    console: &mut dyn Console,
    root: &Path,
    checked: &Checked,
    compat: Compat,
) {
    match checked {
        Checked::Fine => {}
        Checked::Project(report) => console::report(console, report, compat),
        Checked::Modules(modules) => {
            console::report_doc(console, &report::modules_doc(root, modules), compat);
        }
    }
}

/// The terminal style's line for problems in `modules` modules.
pub(crate) fn problems_summary(modules: usize) -> String {
    match modules {
        1 => "Detected problems in 1 module.".to_owned(),
        n => format!("Detected problems in {n} modules."),
    }
}

/// `MakeAppNeedsFileNames` and `MakePkgNeedsExposing`.
fn no_input(personality: Personality, outline: &Outline) -> Report {
    let command = match personality {
        Personality::Nomi => "nomi check".to_owned(),
        Personality::Elm => personality.command(ElmCommand::Make),
    };
    let examples = vcat([
        text(format!("{command} src/Main.elm"))
            .color(Color::Green)
            .indent(4),
        text(format!("{command} src/This.elm src/That.elm"))
            .color(Color::Green)
            .indent(4),
    ]);
    let advice = match outline {
        Outline::App(_) => {
            "I recommend reading through https://guide.elm-lang.org for guidance on what to \
             actually put in those files!"
        }
        Outline::Pkg(_) => {
            "You can also entries to the \"exposed-modules\" list in your elm.json file, and I \
             will try to compile the relevant files."
        }
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
    /// The modules the direct dependencies expose, by name.
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
    let fetched = packages::fetch(Pool::global(), &store, http, compat, &releases);
    let direct: Vec<&Name> = match &project.outline {
        Outline::App(app) => app.direct.keys().collect(),
        Outline::Pkg(pkg) => pkg.deps.keys().collect(),
    };
    let mut exposing = HashMap::new();
    let mut packages = Vec::new();
    for ((name, version), dir) in releases.iter().zip(fetched) {
        let dir =
            dir.map_err(|problem| registry_report::package_problem(name, *version, &problem))?;
        if direct.contains(&name) {
            let Ok(Some(pkg)) = project::package_outline(&dir, compat) else {
                return Err(one_line(
                    "CORRUPT PACKAGE DATA",
                    &format!(
                        "I could not read the elm.json of {name} {version} in {}.",
                        dir.display()
                    ),
                ));
            };
            exposing.insert(name.clone(), pkg.exposed.modules().cloned().collect());
        }
        packages.push((name.clone(), dir));
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
) -> Result<(Foreigns, HashMap<String, Interface>), Report> {
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
    Ok((sources.foreigns, names::visible(&built.exposed(), &direct)))
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
    let owner = match outline {
        Outline::App(_) => Owner::Application,
        Outline::Pkg(pkg) => Owner::Package(pkg.name.clone()),
    };
    let src_dirs: Vec<PathBuf> = (outline.source_dirs().into_iter())
        .map(PathBuf::from)
        .collect();
    Env::new(compat, root, owner, &src_dirs, foreigns).map_err(|e| {
        one_line(
            "PROBLEM READING PROJECT",
            &format!("I could not read the project at {}: {e}", root.display()),
        )
    })
}
