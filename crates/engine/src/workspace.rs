//! The workspace around a directory: the root `nomi.toml` with a
//! `[workspace]`, its member patterns found on disk, and each member's
//! `elm.json` and `nomi.toml` read into the graph `nomi_project` makes.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use nomi_base::{Compat, Report};
use nomi_project::workspace::{Member, Workspace, WorkspaceError};
use nomi_project::{Manifest, Name, Outline, report};
use nomi_registry::{SolverProblem, WorkspaceProblem};

use crate::Vfs;
use crate::console::Personality;
use crate::install::details_problem;
use crate::paths::{ELM_JSON, NOMI_TOML, Root};
use crate::project::{Project, elm_json};
use crate::report::one_line;
use crate::vfs;

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

/// That the project depends on `name`, the member package in `dir`,
/// which nomi doesn't build from its source.
pub(crate) fn member_dependency(name: &Name, dir: &Path) -> Report {
    one_line(
        "MEMBER PACKAGE",
        &format!(
            "This project depends on {name}, the workspace member in {}, and nomi does not \
             build member packages from their source.",
            dir.display(),
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
