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
use nomi_project::workspace::Workspace;
use nomi_project::{ModuleName, Name, Outline, Version};
use nomi_registry::{
    Bundle, Http, Store, details, report as registry_report, workspace as registry_workspace,
};

use crate::artifacts::{Artifacts, Hash, Key};
use crate::console::{self, Console, Personality};
use crate::graph::{Env, Foreigns, Graph, Owner, RootMain};
use crate::install::details_problem;
use crate::names::{ModuleInterface, Packages};
use crate::packages::Resolve;
use crate::project::{self, Project};
use crate::report::{self, ModuleErrors, Names, one_line};
use crate::schedule::Cancelled;
use crate::workspace::{self, Members};
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
    /// What the goal asked for, if nothing was wrong and it asked.
    pub made: Option<Made>,
}

/// What a clean build makes for its goal.
#[derive(Debug)]
pub enum Made {
    /// Each module's interface, checking.
    Interfaces(Vec<ModuleInterface>),
    /// `docs.json`, documenting.
    Docs(String),
    Optimized(Optimized),
}

/// What optimizing a project makes.
#[derive(Clone, Debug)]
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
            made: None,
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
        let made = match goal {
            _ if !matches!(checked, Checked::Fine) => None,
            Goal::Check => Some(Made::Interfaces(walked.interfaces)),
            Goal::Docs => Some(Made::Docs(nomi_compiler::docs::encode(
                env.compat(),
                &walked.docs,
            ))),
            Goal::Optimize => Some(Made::Optimized(Optimized {
                roots: graph.root_mains(),
                graphs: walked.optimized,
                interfaces: walked.interfaces,
            })),
        };
        Ok(Built {
            checked,
            ran: walked.ran,
            compiled: walked.compiled,
            made,
        })
    }
}

/// Shows `checked`: in the terminal style, the reports on standard error,
/// naming files as `names` does, and a line saying how it went on
/// standard output (`toFinalMessage`); in JSON, the report alone, on
/// standard error, as `encodeUgly` writes it.
pub fn show<'a>(
    console: &mut dyn Console,
    names: impl Into<Names<'a>>,
    checked: &Checked,
    compat: Compat,
    style: Style,
) {
    let names = names.into();
    match (checked, style) {
        (Checked::Fine, Style::Terminal) => {
            console.out("No problems in syntax, module names, imports, names or types.\n");
        }
        (Checked::Project(_), Style::Terminal) => show_reports(console, names, checked, compat),
        (Checked::Modules(modules), Style::Terminal) => {
            console.out(&format!("{}\n", problems_summary(modules.len())));
            show_reports(console, names, checked, compat);
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
/// naming files as `names` does, without the line saying how it went.
pub(crate) fn show_reports<'a>(
    console: &mut dyn Console,
    names: impl Into<Names<'a>>,
    checked: &Checked,
    compat: Compat,
) {
    match checked {
        Checked::Fine => {}
        Checked::Project(report) => console::report(console, report, compat),
        Checked::Modules(modules) => {
            console::report_doc(console, &report::modules_doc(names.into(), modules), compat);
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
    /// Every package from the registry or nomi's bundle, and the
    /// directory of its source.
    pub packages: Vec<(Name, PathBuf)>,
    /// nomi's own packages, those of `packages` served from it trusted
    /// with kernel code.
    pub bundle: Bundle,
    /// The member packages of the workspace it builds with, if any.
    pub members: Option<Members>,
}

/// What the project builds with, every release downloaded first, its
/// dependencies resolved as `resolve` says: with the member packages of
/// its workspace, or of `members_of`'s where it is in none.
pub(crate) fn sources(
    cx: &Context<'_, impl Http + Sync>,
    project: &Project,
    personality: Personality,
    resolve: Resolve,
    members_of: Option<&Path>,
) -> Result<Sources, Report> {
    let compat = project.compat;
    let store = packages::store_for(cx.cache, resolve)?;
    let workspace = match resolve {
        Resolve::Upstream => None,
        Resolve::Nomi => {
            let dir = members_of.unwrap_or_else(|| project.root.dir());
            workspace::discover(&cx.vfs.snapshot(), dir).transpose()?
        }
    };
    let artifacts = Artifacts::new(cx.cache);
    let key = details_key(project, resolve, members_of, workspace.as_ref());
    let kept = artifacts
        .get(key)
        .and_then(|bytes| decode_releases(&bytes, compat));
    let mut releases = if let Some(releases) = kept {
        releases
    } else {
        let releases = solve(
            cx,
            &store,
            project,
            personality,
            workspace.as_ref(),
            members_of,
        )?;
        // A cache that can't be written only costs a solve next time.
        let _ = artifacts.put(key, encode_releases(&releases).as_bytes());
        releases
    };
    let members = workspace.and_then(|w| Members::among(&w, &mut releases));
    fetch(cx, &store, project, &releases, members)
}

/// The key the project's solution is kept by. elm keeps a project's
/// dependencies as solved until its elm.json changes (`Details.load`),
/// asking the website nothing; nomi keeps them by what elm.json says, and
/// by what else it resolves them to, so that neither reuses a solution
/// the other can't build.
fn details_key(
    project: &Project,
    resolve: Resolve,
    members_of: Option<&Path>,
    workspace: Option<&Workspace>,
) -> Hash {
    let mut key = Key::new("details")
        .part("compat", project.compat.as_str().as_bytes())
        .part("outline", &project.outline.write())
        .part("resolve", resolve.as_str().as_bytes())
        .part("outside", &[u8::from(members_of.is_some())]);
    for member in workspace.iter().flat_map(|w| w.members()) {
        key = (key.part("member", member.dir.as_os_str().as_encoded_bytes()))
            .part("outline", &member.outline.write());
    }
    key.finish()
}

/// The releases the project's dependencies resolve to: alone, as a member
/// of `workspace`, or with its member packages where the project is
/// `members_of`'s and none of them.
fn solve(
    cx: &Context<'_, impl Http + Sync>,
    store: &Store,
    project: &Project,
    personality: Personality,
    workspace: Option<&Workspace>,
    members_of: Option<&Path>,
) -> Result<Vec<(Name, Version)>, Report> {
    let compat = project.compat;
    let mut registry = env::registry(
        cx.cache,
        store,
        cx.http,
        compat,
        "I need the list of published packages to check your dependencies",
    )?;
    let solution = match (workspace, members_of) {
        (None, _) => details::verify(&mut registry, compat, &project.outline)
            .map_err(|problem| details_problem(&problem, compat, personality)),
        (Some(workspace), None) => {
            registry_workspace::verify(&mut registry, compat, workspace, &project.outline)
                .map_err(|problem| workspace::resolution_problem(&problem, compat, personality))
        }
        (Some(workspace), Some(_)) => {
            let mut members = registry_workspace::Members::new(workspace, &mut registry);
            details::verify(&mut members, compat, &project.outline)
                .map_err(|problem| details_problem(&problem, compat, personality))
        }
    }?;
    Ok((solution.iter())
        .map(|(name, details)| (name.clone(), details.version))
        .collect())
}

/// What the project builds with, `releases` downloaded from `store` and
/// `members` beside them.
fn fetch(
    cx: &Context<'_, impl Http + Sync>,
    store: &Store,
    project: &Project,
    releases: &[(Name, Version)],
    members: Option<Members>,
) -> Result<Sources, Report> {
    let compat = project.compat;
    let fetched = packages::fetch(Pool::global(), store, cx.http, compat, releases);
    let direct: Vec<&Name> = project.outline.direct().collect();
    let mut exposing = HashMap::new();
    for member in members.iter().flat_map(|m| &m.packages) {
        if direct.contains(&&member.outline.name) {
            let modules = member.outline.exposed.modules().cloned().collect();
            exposing.insert(member.outline.name.clone(), modules);
        }
    }
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
        bundle: store.bundle().clone(),
        members,
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
    cx: &Context<'_, impl Http + Sync>,
    project: &Project,
    personality: Personality,
) -> Result<(Foreigns, HashMap<String, Interface>), Report> {
    let sources = sources(cx, project, personality, Resolve::of(personality), None)?;
    let artifacts = Artifacts::new(cx.cache);
    let built = objects::build(
        project.compat,
        &sources.packages,
        &sources.bundle,
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
