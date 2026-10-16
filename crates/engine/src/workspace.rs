//! The workspace around a directory: the root `nomi.toml` with a
//! `[workspace]`, its member patterns found on disk, and each member's
//! `elm.json` and `nomi.toml` read into the graph `nomi_project` makes;
//! and the member packages a project uses, built from their source.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use nomi_base::{Compat, Platform, Report, lock};
use nomi_project::outline::PkgOutline;
use nomi_project::workspace::{Member, Workspace, WorkspaceError};
use nomi_project::{Entry, Manifest, ModuleName, Name, Outline, Version, report};
use nomi_registry::{Http, SolverProblem, WorkspaceProblem};

use crate::artifacts::Artifacts;
use crate::check::{self, Build, Checked, Goal, Made, Style, env_of, packages_problem};
use crate::console::{self, Console, Personality};
use crate::graph::Foreigns;
use crate::install::details_problem;
use crate::make::{self, FfiModule, Make, NamedIn, Shared};
use crate::names::{Building, Packages};
use crate::objects::{self, Memo, PackageBuild};
use crate::packages::Resolve;
use crate::paths::{ELM_JSON, NOMI_TOML, Root};
use crate::project::{Project, elm_json, package_outline};
use crate::report::one_line;
use crate::schedule::Cancelled;
use crate::timings;
use crate::{Context, Snapshot, javascript, run, vfs};

/// The workspace around `dir`, if there is one: the nearest of its
/// ancestors whose `nomi.toml` has a `[workspace]`, unless the project
/// around `dir` is below that root and none of its members, in which
/// case the project stands alone there and the search goes on above.
pub fn discover(snapshot: &Snapshot, dir: &Path) -> Option<Result<Workspace, Report>> {
    let project = (dir.ancestors()).find(|d| snapshot.is_file(&d.join(ELM_JSON)));
    for root in dir.ancestors() {
        let path = root.join(NOMI_TOML);
        if !snapshot.is_file(&path) {
            continue;
        }
        let manifest = match manifest(snapshot, &path, NOMI_TOML) {
            Ok(manifest) => manifest,
            Err(report) => return Some(Err(report)),
        };
        let Some(workspace) = &manifest.workspace else {
            continue;
        };
        let below = project
            .and_then(|project| project.strip_prefix(root).ok())
            .filter(|dir| !dir.as_os_str().is_empty());
        if let Some(dir) = below
            && !workspace.members.iter().any(|m| m.pattern.matches(dir))
        {
            continue;
        }
        return Some(load(snapshot, root, manifest));
    }
    None
}

/// The project at `root`, standing alone, as a workspace of its one
/// member, where its `nomi.toml` has entries for a build to make.
pub(crate) fn alone(snapshot: &Snapshot, root: &Path) -> Option<Result<Workspace, Report>> {
    let path = root.join(NOMI_TOML);
    if !snapshot.is_file(&path) {
        return None;
    }
    let manifest = match manifest(snapshot, &path, NOMI_TOML) {
        Ok(manifest) if manifest.entries.is_empty() => return None,
        Ok(manifest) => manifest,
        Err(report) => return Some(Err(report)),
    };
    Some(load(snapshot, root, manifest))
}

/// The directory of the member in `dir`, from the root of `workspace`.
pub(crate) fn member_dir(workspace: &Workspace, dir: &Path) -> PathBuf {
    dir.strip_prefix(workspace.root()).unwrap_or(dir).to_owned()
}

/// The workspace whose root `nomi.toml`, in `root`, holds `manifest`.
fn load(snapshot: &Snapshot, root: &Path, manifest: Manifest) -> Result<Workspace, Report> {
    let has_project = snapshot.is_file(&root.join(ELM_JSON));
    if !has_project && !manifest.entries.is_empty() {
        return Err(entry_without_project());
    }
    let mut dirs = BTreeSet::new();
    for member in manifest.workspace.iter().flat_map(|w| &w.members) {
        let found = (member.pattern)
            .expand(|dir| subdirs(&root.join(dir)))
            .map_err(|e| {
                one_line(
                    "PROBLEM READING MEMBERS",
                    &format!(
                        "I could not list the members `{}` names: {e}",
                        member.pattern
                    ),
                )
            })?;
        for dir in found {
            if snapshot.is_file(&root.join(&dir).join(ELM_JSON)) {
                dirs.insert(dir);
            } else if member.pattern.is_literal() {
                return Err(missing_member(&dir));
            }
        }
    }
    let mut members = Vec::with_capacity(dirs.len() + 1);
    if has_project {
        members.push(Member::new(
            PathBuf::new(),
            outline(snapshot, root, Path::new(""))?,
            Some(manifest),
        ));
    }
    for dir in dirs {
        let path = root.join(&dir).join(NOMI_TOML);
        let shown = shown(&dir.join(NOMI_TOML));
        let manifest = (snapshot.is_file(&path))
            .then(|| self::manifest(snapshot, &path, &shown))
            .transpose()?;
        let outline = outline(snapshot, root, &dir)?;
        members.push(Member::new(dir, outline, manifest));
    }
    Workspace::new(root.to_owned(), members).map_err(|e| problem(&e))
}

/// The names of the directories in `dir`; none where it isn't one, so
/// that `packages/*` names nothing until `packages` exists.
fn subdirs(dir: &Path) -> io::Result<Vec<OsString>> {
    match vfs::subdirs(dir) {
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(Vec::new())
        }
        listed => listed,
    }
}

/// The outline of the member in `dir`, below `root`.
fn outline(snapshot: &Snapshot, root: &Path, dir: &Path) -> Result<Outline, Report> {
    let project = Root::new(root.join(dir));
    let named = |mut report: Report| {
        report.path = Some(shown(&dir.join(ELM_JSON)));
        report
    };
    let src = elm_json(snapshot, &project).map_err(named)?;
    let project = Project::decode(&project, &src, Personality::Nomi).map_err(named)?;
    Ok(project.outline)
}

/// The `nomi.toml` at `path`, shown as `shown`.
fn manifest(snapshot: &Snapshot, path: &Path, shown: &str) -> Result<Manifest, Report> {
    let src = snapshot.read_bytes(path).map_err(|e| {
        one_line(
            "PROBLEM READING NOMI.TOML",
            &format!("I could not read {shown}: {e}"),
        )
    })?;
    Manifest::parse(&src).map_err(|error| {
        let mut report = report::manifest(&error, &src, Compat::LATEST);
        report.path = Some(shown.to_owned());
        report
    })
}

/// `path`, relative to the root, as the root's own reports show it.
fn shown(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        ".".to_owned()
    } else {
        path.display().to_string()
    }
}

fn problem(error: &WorkspaceError) -> Report {
    match error {
        WorkspaceError::Nested(dir) => one_line(
            "NESTED WORKSPACE",
            &format!(
                "{} has a [workspace] of its own, but {} is a member of the workspace in \
                 nomi.toml. A workspace can't hold another, so remove one of the two \
                 [workspace] tables.",
                shown(&dir.join(NOMI_TOML)),
                shown(dir),
            ),
        ),
        WorkspaceError::Reserved { dir, name } => one_line(
            "RESERVED NAME",
            &format!(
                "The member {} is the package {name}, but packages by elm, elm-explorations \
                 and nomi may hold kernel code, so they come only from the package registry \
                 and nomi. Give it a name of your own in its elm.json.",
                shown(dir),
            ),
        ),
        WorkspaceError::Duplicate {
            name,
            first,
            second,
        } => one_line(
            "DUPLICATE MEMBER",
            &format!(
                "The members {} and {} are both the package {name}, but each member package \
                 needs a name of its own, since that is how the others depend on it.",
                shown(first),
                shown(second),
            ),
        ),
        WorkspaceError::Cycle(names) => {
            let mut cycle: Vec<String> = names.iter().map(ToString::to_string).collect();
            cycle.push(names[0].to_string());
            one_line(
                "MEMBER CYCLE",
                &format!(
                    "The member packages depend on each other in a cycle: {}. A package \
                     can't depend on itself, even through others.",
                    cycle.join(", which depends on "),
                ),
            )
        }
    }
}

/// Why the members don't resolve together, as `personality` tells it.
pub(crate) fn resolution_problem(
    problem: &WorkspaceProblem<SolverProblem>,
    compat: Compat,
    personality: Personality,
) -> Report {
    let elm_json = |dir: &Path| shown(&dir.join(ELM_JSON));
    match problem {
        WorkspaceProblem::MixedElm { first, second } => one_line(
            "MIXED ELM VERSIONS",
            &format!(
                "{} is for elm {}, but {} is for elm {}. A workspace builds with one \
                 compiler, so give every application the same \"elm-version\".",
                elm_json(&first.dir),
                first.version,
                elm_json(&second.dir),
                second.version,
            ),
        ),
        WorkspaceProblem::Disagree {
            name,
            first,
            second,
        } => one_line(
            "MEMBERS DISAGREE",
            &format!(
                "{} pins {name} {}, but {} pins {name} {}. The members of a workspace build \
                 with one release of each package, so pin the same one in both.",
                elm_json(&first.dir),
                first.version,
                elm_json(&second.dir),
                second.version,
            ),
        ),
        WorkspaceProblem::MemberVersion { name, member, by } => one_line(
            "MEMBER VERSION",
            &format!(
                "{} pins {name} {}, but {name} is the workspace member {} at {}. Pin the \
                 member's version.",
                elm_json(&by.dir),
                by.version,
                shown(&member.dir),
                member.version,
            ),
        ),
        WorkspaceProblem::MemberRange {
            name,
            member,
            by,
            range,
        } => one_line(
            "MEMBER VERSION",
            &format!(
                "{} depends on {name} {range}, but {name} is the workspace member {} at {}. \
                 Give it a range that includes the member's version.",
                elm_json(by),
                shown(&member.dir),
                member.version,
            ),
        ),
        WorkspaceProblem::Details(problem) => details_problem(problem, compat, personality),
    }
}

/// The member packages a project builds with, and the root of their
/// workspace, which their reports name paths from.
#[derive(Debug)]
pub(crate) struct Members {
    pub root: PathBuf,
    /// In the order they build, each after those it depends on.
    pub packages: Vec<MemberPackage>,
}

/// A member package: its directory and its `elm.json`.
#[derive(Debug)]
pub(crate) struct MemberPackage {
    pub dir: PathBuf,
    pub outline: PkgOutline,
}

impl Members {
    /// The member packages of `workspace` among `releases`, which keeps
    /// the others; `None` if there are none.
    pub(crate) fn among(
        workspace: &Workspace,
        releases: &mut Vec<(Name, Version)>,
    ) -> Option<Members> {
        let members = Members::named(workspace, |name| releases.iter().any(|(n, _)| n == name))?;
        releases.retain(|(name, _)| !members.has(name));
        Some(members)
    }

    /// The member packages of `workspace` that `deps` names, and those
    /// they depend on in turn; `None` if there are none.
    pub(crate) fn reached<'n>(
        workspace: &Workspace,
        deps: impl IntoIterator<Item = &'n Name>,
    ) -> Option<Members> {
        let mut used: BTreeSet<&Name> = deps.into_iter().collect();
        // Each member after those it depends on: backwards, a dependent
        // comes first.
        for member in workspace.members().iter().rev() {
            if let Outline::Pkg(pkg) = &member.outline
                && used.contains(&pkg.name)
            {
                used.extend(pkg.deps.keys());
            }
        }
        Members::named(workspace, |name| used.contains(name))
    }

    /// The member packages of `workspace` whose names `wanted` takes, in
    /// order; `None` if there are none.
    fn named(workspace: &Workspace, wanted: impl Fn(&Name) -> bool) -> Option<Members> {
        let packages: Vec<MemberPackage> = (workspace.members().iter())
            .filter_map(|member| match &member.outline {
                Outline::Pkg(pkg) if wanted(&pkg.name) => Some(MemberPackage {
                    dir: workspace.root().join(&member.dir),
                    outline: pkg.clone(),
                }),
                Outline::Pkg(_) | Outline::App(_) => None,
            })
            .collect();
        (!packages.is_empty()).then(|| Members {
            root: workspace.root().to_owned(),
            packages,
        })
    }

    /// Whether `name` is one of them.
    pub(crate) fn has(&self, name: &Name) -> bool {
        self.packages.iter().any(|p| p.outline.name == *name)
    }
}

/// How member packages build: from their source, through the module
/// build, against the registry's packages they reach.
#[derive(Debug)]
pub(crate) struct MemberBuild<'a> {
    pub snapshot: &'a Snapshot,
    pub compat: Compat,
    pub personality: Personality,
    /// The registry's packages of the project, and their directories.
    pub packages: &'a [(Name, PathBuf)],
    /// What earlier builds left, reused.
    pub artifacts: Option<&'a Artifacts>,
    /// Each registry package's build, shared with the builds before and
    /// after.
    pub memo: &'a Mutex<Memo>,
    /// What the project's build makes: optimizing, it needs a member's
    /// graph and types as well as its exposed interfaces.
    pub goal: Goal,
}

/// Member packages built.
#[derive(Debug, Default)]
pub(crate) struct MembersBuilt {
    pub builds: BTreeMap<Name, Arc<PackageBuild>>,
    /// The modules checked rather than taken from the artifact cache.
    pub ran: Vec<PathBuf>,
}

impl MemberBuild<'_> {
    /// Each of `members` built in order, or taken from `known`, the
    /// command's builds before; or what was wrong with the first that
    /// failed. Cancelled once the snapshot is stale.
    pub(crate) fn run(
        &self,
        members: &Members,
        known: &Mutex<BTreeMap<(Name, Compat), Arc<PackageBuild>>>,
    ) -> Result<Result<MembersBuilt, Checked>, Cancelled> {
        let mut built = MembersBuilt::default();
        for member in &members.packages {
            let name = &member.outline.name;
            let key = (name.clone(), self.compat);
            let known_build = lock(known).get(&key).cloned();
            let build = if let Some(build) = known_build {
                build
            } else {
                let build = match self.member(members, member, &mut built)? {
                    Ok(build) => Arc::new(build),
                    Err(checked) => return Ok(Err(checked)),
                };
                lock(known).insert(key, build.clone());
                build
            };
            built.builds.insert(name.clone(), build);
        }
        Ok(Ok(built))
    }

    /// [`MemberBuild::run`] in a command, which nothing revises the files
    /// during, with its builds before in `shared`.
    pub(crate) fn in_command(
        &self,
        members: &Members,
        shared: &Shared,
    ) -> Result<MembersBuilt, Checked> {
        (self.run(members, &shared.members)).expect("nothing revises the files while nomi builds")
    }

    /// `member` built as a project of its own, the members before it in
    /// `built`.
    pub(crate) fn member(
        &self,
        members: &Members,
        member: &MemberPackage,
        built: &mut MembersBuilt,
    ) -> Result<Result<PackageBuild, Checked>, Cancelled> {
        let compat = self.compat;
        let pkg = &member.outline;
        let exposed: Vec<ModuleName> = pkg.exposed.modules().cloned().collect();
        if exposed.is_empty() {
            return Ok(Ok(PackageBuild::default()));
        }
        let reached = match self.reached(pkg, &built.builds) {
            Ok(reached) => reached,
            Err(report) => return Ok(Err(Checked::Project(report))),
        };
        let direct: Vec<(Name, Vec<ModuleName>)> = (pkg.deps.keys())
            .filter_map(|name| {
                let outline = (reached.get(name).map(|(_, outline)| outline)).or_else(|| {
                    let member = members.packages.iter().find(|m| m.outline.name == *name);
                    Some(&member?.outline)
                })?;
                Some((name.clone(), outline.exposed.modules().cloned().collect()))
            })
            .collect();
        let outline = Outline::Pkg(pkg.clone());
        let env = match env_of(&member.dir, &outline, compat, Foreigns::new(direct.clone())) {
            Ok(env) => env,
            Err(report) => return Ok(Err(Checked::Project(report))),
        };
        let sources = (reached.into_iter()).map(|(name, (dir, _))| (name, dir));
        let building = Building::new(
            compat,
            sources.collect(),
            built.builds.clone(),
            self.artifacts,
            self.memo,
            direct,
        );
        let build = Build {
            snapshot: self.snapshot,
            env: &env,
            outline: &outline,
            paths: &[],
            personality: self.personality,
            packages: Packages::Building(&building),
            artifacts: self.artifacts,
        };
        let goal = match self.goal {
            Goal::Optimize => Goal::Optimize,
            Goal::Check | Goal::Docs => Goal::Check,
        };
        let mut modules = build.run(goal)?;
        built.ran.append(&mut modules.ran);
        if let Err(why) = building.finish() {
            return Ok(Err(Checked::Project(packages_problem(&why))));
        }
        let (interfaces, graphs) = match modules.made {
            Some(Made::Interfaces(interfaces)) => (interfaces, None),
            Some(Made::Optimized(optimized)) => (optimized.interfaces, Some(optimized.graphs)),
            Some(Made::Docs(_)) | None => return Ok(Err(modules.checked)),
        };
        Ok(Ok(objects::member_build(
            compat,
            &pkg.name,
            &exposed,
            &interfaces,
            graphs,
        )))
    }

    /// The registry's packages `pkg` reaches, the members in `built`
    /// standing for themselves, and each one's directory and `elm.json`.
    fn reached(
        &self,
        pkg: &PkgOutline,
        built: &BTreeMap<Name, Arc<PackageBuild>>,
    ) -> Result<BTreeMap<Name, (PathBuf, PkgOutline)>, Report> {
        let mut reached = BTreeMap::new();
        let mut todo: Vec<Name> = pkg.deps.keys().cloned().collect();
        while let Some(name) = todo.pop() {
            if reached.contains_key(&name) || built.contains_key(&name) {
                continue;
            }
            let Some((_, dir)) = self.packages.iter().find(|(n, _)| *n == name) else {
                continue;
            };
            let outline = match package_outline(dir, self.compat) {
                Ok(Some(outline)) => outline,
                Ok(None) => {
                    return Err(packages_problem(&format!(
                        "{name}'s elm.json isn't a package's"
                    )));
                }
                Err(e) => return Err(packages_problem(&format!("{name}: {e}"))),
            };
            todo.extend(outline.deps.keys().cloned());
            reached.insert(name, (dir.clone(), outline));
        }
        Ok(reached)
    }
}

/// How a member's build went.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Built, checking these modules rather than taking them from the
    /// artifact cache.
    Built(Vec<PathBuf>),
    /// Not built, and what was wrong shown.
    Failed,
    /// Not built, as a member package it depends on wasn't.
    Blocked,
}

/// What a workspace's build makes of its members.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Making {
    /// Their modules checked, as `nomi check` checks them.
    Check,
    /// Their entries' code, of this kind, as `nomi build` writes it.
    Code(javascript::Kind),
    /// Their entries' code as `nomi run` writes it: DEV mode's, without
    /// its advice to optimize.
    Run,
}

/// Builds the members of `workspace` whose directories `dirs` gives, and
/// the member packages they depend on, in order, as `making` asks: a
/// package's exposed modules and then each entry of a member's
/// `nomi.toml`, with one artifact cache and one build of each package;
/// shows what was wrong, naming paths from the root; and gives how each
/// member built went, in the members' order. A member is blocked when a
/// member package it depends on fails, and says nothing, as a module whose
/// import failed doesn't. With `NOMI_TIMINGS`, where the time went
/// follows on standard error.
pub fn build<'w>(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    workspace: &'w Workspace,
    dirs: &[PathBuf],
    making: Making,
) -> Vec<(&'w Member, Outcome)> {
    builds(cx, console, workspace, dirs, making).outcomes
}

/// What [`builds`] made of a workspace's members.
#[derive(Debug)]
pub(crate) struct Builds<'w> {
    /// How each member's build went, in the members' order.
    pub outcomes: Vec<(&'w Member, Outcome)>,
    /// Each entry whose code was written, in the members' order.
    pub entries: Vec<EntryBuilt<'w>>,
}

/// An entry of a member's `nomi.toml` whose code a build wrote.
#[derive(Debug)]
pub(crate) struct EntryBuilt<'w> {
    pub member: &'w Member,
    pub entry: &'w Entry,
    /// Where the code went, absolute.
    pub output: PathBuf,
    /// Where the code runs, as the entry or its `main` decided.
    pub platform: Platform,
    /// The module whose `main` the code starts.
    pub module: ModuleName,
}

/// [`build`], with each entry it wrote.
pub(crate) fn builds<'w>(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    workspace: &'w Workspace,
    dirs: &[PathBuf],
    making: Making,
) -> Builds<'w> {
    let shared = Shared::new(cx.cache, Some(workspace.root()));
    builds_in(cx, console, &shared, workspace, dirs, making)
}

/// [`builds`] with what earlier builds in `shared` left, which a watch
/// keeps from one build to the next.
pub(crate) fn builds_in<'w>(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    shared: &Shared,
    workspace: &'w Workspace,
    dirs: &[PathBuf],
    making: Making,
) -> Builds<'w> {
    let started = Instant::now();
    let wanted = with_deps(workspace, dirs);
    let mut outcomes: Vec<(&Member, Outcome)> = Vec::with_capacity(wanted.len());
    let mut entries = Vec::new();
    for member in (workspace.members().iter()).filter(|m| wanted.contains(m.dir.as_path())) {
        let blocked = workspace.deps(member).any(|dep| {
            let (_, outcome) = (outcomes.iter().find(|(m, _)| m.dir == dep.dir))
                .expect("a member's dependency builds before it");
            !matches!(outcome, Outcome::Built(_))
        });
        let outcome = if blocked {
            Outcome::Blocked
        } else {
            let releases = releases(workspace, &wanted, member);
            let (outcome, mut wrote) =
                built(cx, console, shared, workspace, member, &releases, making);
            entries.append(&mut wrote);
            outcome
        };
        outcomes.push((member, outcome));
    }
    make::trim(cx.cache);
    if timings::on() {
        console.err(&timings::report(started.elapsed()));
    }
    Builds { outcomes, entries }
}

/// The members of `workspace` in `dirs`, and the member packages they
/// depend on, by their directories.
pub(crate) fn with_deps<'w>(workspace: &'w Workspace, dirs: &'w [PathBuf]) -> BTreeSet<&'w Path> {
    let mut wanted: BTreeSet<&Path> = dirs.iter().map(PathBuf::as_path).collect();
    // Each member after those it depends on: backwards, a dependent
    // comes first.
    for member in workspace.members().iter().rev() {
        if wanted.contains(member.dir.as_path()) {
            wanted.extend(workspace.deps(member).map(|dep| dep.dir.as_path()));
        }
    }
    wanted
}

/// The releases the member package `member` is built for: those of the
/// members in `wanted` that use it, an application's its own and a
/// package's those it is built for, or its own when none there uses it.
fn releases(workspace: &Workspace, wanted: &BTreeSet<&Path>, member: &Member) -> BTreeSet<Compat> {
    let users = (workspace.members().iter()).filter(|user| {
        wanted.contains(user.dir.as_path()) && workspace.deps(user).any(|d| d.dir == member.dir)
    });
    let mut releases = BTreeSet::new();
    for user in users {
        match user.outline {
            Outline::App(_) => {
                releases.insert(release(user));
            }
            Outline::Pkg(_) => releases.append(&mut self::releases(workspace, wanted, user)),
        }
    }
    if releases.is_empty() {
        releases.insert(release(member));
    }
    releases
}

/// The release `member` alone is built as.
fn release(member: &Member) -> Compat {
    member.outline.compat().unwrap_or(Compat::LATEST)
}

/// Builds `member` of `workspace`: a package's exposed modules as its
/// dependents see them, for each of `releases`, then each of its entries;
/// how it went, and the entries whose code it wrote.
fn built<'w>(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    shared: &Shared,
    workspace: &Workspace,
    member: &'w Member,
    releases: &BTreeSet<Compat>,
    making: Making,
) -> (Outcome, Vec<EntryBuilt<'w>>) {
    let dir = workspace.root().join(&member.dir);
    let cx = Context { dir: &dir, ..*cx };
    let (goal, verb) = match making {
        Making::Check => (Goal::Check, "Checking"),
        Making::Code(_) | Making::Run => (Goal::Optimize, "Building"),
    };
    // A project alone is the one member, at the root, and needs no name.
    if !member.dir.as_os_str().is_empty() {
        console.out(&format!("{verb} {}\n", member.dir.display()));
    }
    let mut ran = match &member.outline {
        Outline::Pkg(pkg) => {
            match package(&cx, console, shared, workspace.root(), pkg, releases, goal) {
                Some(ran) => ran,
                None => return (Outcome::Failed, Vec::new()),
            }
        }
        Outline::App(_) => Vec::new(),
    };
    let mut entries = Vec::new();
    for entry in member.manifest.iter().flat_map(|m| &m.entries) {
        let make = match entry_make(&dir, entry, making) {
            Ok(make) => make,
            Err(report) => {
                console::report(console, &report, Compat::LATEST);
                return (Outcome::Failed, entries);
            }
        };
        let Some(mut wrote) = make::made(&cx, console, &make, shared) else {
            return (Outcome::Failed, entries);
        };
        ran.append(&mut wrote.ran);
        if let Some(code) = wrote.code {
            entries.push(EntryBuilt {
                member,
                entry,
                output: code.output,
                platform: code.platform,
                module: (code.mains.into_iter().next()).expect("an entry has one main"),
            });
        }
    }
    (Outcome::Built(ran), entries)
}

/// The member package `pkg`, in the directory, built into `shared` for
/// `goal` as its dependents see it, as each of `releases` builds it; the
/// modules checked, or `None` once what was wrong is shown, as the
/// release that found it reports it, its paths named from `root`.
fn package(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    shared: &Shared,
    root: &Path,
    pkg: &PkgOutline,
    releases: &BTreeSet<Compat>,
    goal: Goal,
) -> Option<Vec<PathBuf>> {
    let mut fail = |checked, compat| {
        check::show(console, root, &checked, compat, Style::Terminal);
        None
    };
    let project = Root::new(cx.dir.to_owned());
    let loaded = match make::load(cx, &project, Personality::Nomi, Resolve::Nomi, None) {
        Ok(loaded) => loaded,
        Err((report, compat)) => return fail(Checked::Project(report), compat),
    };
    let mut members = (loaded.sources.members).unwrap_or_else(|| Members {
        root: root.to_owned(),
        packages: Vec::new(),
    });
    members.packages.push(MemberPackage {
        dir: cx.dir.to_owned(),
        outline: pkg.clone(),
    });
    let snapshot = cx.vfs.snapshot();
    let mut ran = Vec::new();
    for &compat in releases {
        let build = MemberBuild {
            snapshot: &snapshot,
            compat,
            personality: Personality::Nomi,
            packages: &loaded.sources.packages,
            artifacts: Some(&shared.artifacts),
            memo: &shared.memo,
            goal,
        };
        match build.in_command(&members, shared) {
            Ok(mut built) => ran.append(&mut built.ran),
            Err(checked) => return fail(checked, compat),
        }
    }
    check::show(
        console,
        root,
        &Checked::Fine,
        loaded.compat,
        Style::Terminal,
    );
    Some(ran)
}

/// What building `entry` of the member in `dir` asks for, as `making`
/// asks: `nomi check` of its `main`, or what `nomi build` or `nomi run`
/// makes of its `main`, platform and FFI module, written to its `output` or, without
/// one, to a file of its own in `elm-stuff`.
fn entry_make(dir: &Path, entry: &Entry, making: Making) -> Result<Make, Report> {
    let kind = match making {
        Making::Check => {
            return Ok(Make::check(
                vec![entry.main.clone().into()],
                Style::Terminal,
            ));
        }
        Making::Code(kind) => kind,
        Making::Run => javascript::Kind::Dev,
    };
    let output = entry.output.clone().unwrap_or_else(|| {
        let path = run::default_output(dir, &dir.join(&entry.main), run::extension(entry));
        path.to_string_lossy().into_owned()
    });
    let make = Make::build(
        vec![entry.main.clone().into()],
        Some(&output),
        entry.platform,
    )
    .ok_or_else(|| bad_output(&entry.main, &output))?;
    let target = javascript::Target {
        quiet: making == Making::Run,
        ..make.target.clone()
    };
    Ok(Make {
        kind: Ok(kind),
        target,
        output_by_platform: entry.output.is_none(),
        ffi: (entry.ffi.as_ref()).map(|path| FfiModule {
            path: path.into(),
            named_in: NamedIn::Entry(entry.main.clone()),
        }),
        ..make
    })
}

fn bad_output(main: &str, output: &str) -> Report {
    one_line(
        "BAD OUTPUT",
        &format!(
            "The entry {main} is written to {output}, but nomi writes code only to a file \
             ending in .js, .mjs or .html, or to /dev/null."
        ),
    )
}

fn missing_member(dir: &Path) -> Report {
    one_line(
        "MISSING MEMBER",
        &format!(
            "nomi.toml lists the member {}, but there is no elm.json in it. A member is an \
             application or a package with an elm.json of its own.",
            shown(dir),
        ),
    )
}

fn entry_without_project() -> Report {
    one_line(
        "ENTRY WITHOUT PROJECT",
        "The workspace's nomi.toml has [[entry]] tables, but there is no elm.json beside it, \
         so there is no project for them to build. Move each entry to the nomi.toml of the \
         member it belongs to.",
    )
}
