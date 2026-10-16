//! The workspace around a directory: the root `nomi.toml` with a
//! `[workspace]`, its member patterns found on disk, and each member's
//! `elm.json` and `nomi.toml` read into the graph `nomi_project` makes;
//! and the member packages a project uses, built from their source.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nomi_base::{Compat, Report};
use nomi_project::outline::PkgOutline;
use nomi_project::workspace::{Member, Workspace, WorkspaceError};
use nomi_project::{Manifest, ModuleName, Name, Outline, Version, report};
use nomi_registry::{SolverProblem, WorkspaceProblem};

use crate::artifacts::Artifacts;
use crate::check::{Build, Checked, Goal, Made, env_of, packages_problem};
use crate::console::Personality;
use crate::graph::Foreigns;
use crate::install::details_problem;
use crate::names::{Building, Packages};
use crate::objects::{self, Memo, PackageBuild};
use crate::paths::{ELM_JSON, NOMI_TOML, Root};
use crate::project::{Project, elm_json, package_outline};
use crate::report::one_line;
use crate::{Snapshot, Vfs, vfs};

/// The workspace around `dir`, if there is one: the nearest of its
/// ancestors whose `nomi.toml` has a `[workspace]`, unless the project
/// around `dir` is below that root and none of its members, in which
/// case the project stands alone there and the search goes on above.
pub fn discover(vfs: &Vfs, dir: &Path) -> Option<Result<Workspace, Report>> {
    let project = Root::find(vfs, dir);
    for root in dir.ancestors() {
        let path = root.join(NOMI_TOML);
        if !vfs.is_file(&path) {
            continue;
        }
        let manifest = match manifest(vfs, &path, NOMI_TOML) {
            Ok(manifest) => manifest,
            Err(report) => return Some(Err(report)),
        };
        let Some(workspace) = &manifest.workspace else {
            continue;
        };
        let below = (project.as_ref())
            .and_then(|project| project.dir().strip_prefix(root).ok())
            .filter(|dir| !dir.as_os_str().is_empty());
        if let Some(dir) = below
            && !workspace.members.iter().any(|m| m.pattern.matches(dir))
        {
            continue;
        }
        return Some(load(vfs, root, manifest));
    }
    None
}

/// The workspace whose root `nomi.toml`, in `root`, holds `manifest`.
fn load(vfs: &Vfs, root: &Path, manifest: Manifest) -> Result<Workspace, Report> {
    let has_project = vfs.is_file(&root.join(ELM_JSON));
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
            if vfs.is_file(&root.join(&dir).join(ELM_JSON)) {
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
            outline(vfs, root, Path::new(""))?,
            Some(manifest),
        ));
    }
    for dir in dirs {
        let path = root.join(&dir).join(NOMI_TOML);
        let shown = shown(&dir.join(NOMI_TOML));
        let manifest = (vfs.is_file(&path))
            .then(|| self::manifest(vfs, &path, &shown))
            .transpose()?;
        let outline = outline(vfs, root, &dir)?;
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
fn outline(vfs: &Vfs, root: &Path, dir: &Path) -> Result<Outline, Report> {
    let project = Root::new(root.join(dir));
    let named = |mut report: Report| {
        report.path = Some(shown(&dir.join(ELM_JSON)));
        report
    };
    let src = elm_json(vfs, &project).map_err(named)?;
    let project = Project::decode(&project, &src, Personality::Nomi).map_err(named)?;
    Ok(project.outline)
}

/// The `nomi.toml` at `path`, shown as `shown`.
fn manifest(vfs: &Vfs, path: &Path, shown: &str) -> Result<Manifest, Report> {
    let src = vfs.read_bytes(path).map_err(|e| {
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
        let packages: Vec<MemberPackage> = (workspace.members().iter())
            .filter_map(|member| match &member.outline {
                Outline::Pkg(pkg) if releases.iter().any(|(name, _)| *name == pkg.name) => {
                    Some(MemberPackage {
                        dir: workspace.root().join(&member.dir),
                        outline: pkg.clone(),
                    })
                }
                Outline::Pkg(_) | Outline::App(_) => None,
            })
            .collect();
        if packages.is_empty() {
            return None;
        }
        releases.retain(|(name, _)| packages.iter().all(|p| p.outline.name != *name));
        Some(Members {
            root: workspace.root().to_owned(),
            packages,
        })
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
    pub artifacts: &'a Artifacts,
    pub memo: &'a Mutex<Memo>,
    /// What the project's build makes: optimizing, it needs a member's
    /// graph and types as well as its exposed interfaces.
    pub goal: Goal,
}

impl MemberBuild<'_> {
    /// Each of `members` built in order, by name; or what was wrong with
    /// the first that failed.
    pub(crate) fn run(
        &self,
        members: &Members,
    ) -> Result<BTreeMap<Name, Arc<PackageBuild>>, Checked> {
        let mut built = BTreeMap::new();
        for member in &members.packages {
            let build = self.member(members, member, &built)?;
            built.insert(member.outline.name.clone(), Arc::new(build));
        }
        Ok(built)
    }

    /// `member` built as a project of its own, the members before it in
    /// `built`.
    fn member(
        &self,
        members: &Members,
        member: &MemberPackage,
        built: &BTreeMap<Name, Arc<PackageBuild>>,
    ) -> Result<PackageBuild, Checked> {
        let compat = self.compat;
        let pkg = &member.outline;
        let exposed: Vec<ModuleName> = pkg.exposed.modules().cloned().collect();
        if exposed.is_empty() {
            return Ok(PackageBuild::default());
        }
        let reached = self.reached(pkg, built).map_err(Checked::Project)?;
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
        let env = env_of(&member.dir, &outline, compat, Foreigns::new(direct.clone()))
            .map_err(Checked::Project)?;
        let sources = (reached.into_iter()).map(|(name, (dir, _))| (name, dir));
        let building = Building::new(
            compat,
            sources.collect(),
            built.clone(),
            Some(self.artifacts),
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
            artifacts: Some(self.artifacts),
        };
        let goal = match self.goal {
            Goal::Optimize => Goal::Optimize,
            Goal::Check | Goal::Docs => Goal::Check,
        };
        let modules = (build.run(goal)).expect("nothing revises the files while make builds");
        if let Err(why) = building.finish() {
            return Err(Checked::Project(packages_problem(&why)));
        }
        let (interfaces, graphs) = match modules.made {
            Some(Made::Interfaces(interfaces)) => (interfaces, None),
            Some(Made::Optimized(optimized)) => (optimized.interfaces, Some(optimized.graphs)),
            Some(Made::Docs(_)) | None => return Err(modules.checked),
        };
        Ok(objects::member_build(
            compat,
            &pkg.name,
            &exposed,
            &interfaces,
            graphs,
        ))
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
