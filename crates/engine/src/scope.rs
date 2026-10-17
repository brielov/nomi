//! Which projects a command works on: the project around the directory,
//! the members of the workspace around it that `-p` names, or, outside
//! every member, all of them.

use std::path::{Path, PathBuf};

use nomi_base::{Compat, Report};
use nomi_project::Outline;
use nomi_project::workspace::{Member, Unmatched, Workspace};
use nomi_registry::Http;

use crate::check::Style;
use crate::console::{self, Console};
use crate::make::{self, Make, Output};
use crate::paths::Root;
use crate::report::one_line;
use crate::workspace::{self, Making, Outcome};
use crate::{Context, Snapshot};

/// The projects a command works on.
#[derive(Debug)]
pub enum Scope {
    /// The project around the directory, alone or the member it is in.
    Project,
    /// The members `-p` named, by their directories from the root, in
    /// the workspace's order.
    Named(Workspace, Vec<PathBuf>),
    /// Every member: the directory is in the workspace but in none of
    /// them.
    Every(Workspace),
}

/// What `specs`, the values of `-p`, ask for from the directory: those
/// members of the workspace around it, or, without any, the project
/// around it if there is one in the workspace, else every member.
pub fn find(cx: &Context<'_, impl Http>, specs: &[String]) -> Result<Scope, Report> {
    let snapshot = cx.vfs.snapshot();
    let found = workspace::discover(&snapshot, cx.dir);
    if specs.is_empty() {
        let workspace = match found {
            None => return Ok(Scope::Project),
            Some(found) => found?,
        };
        let inside = (workspace.members().iter()).any(|member| {
            !member.dir.as_os_str().is_empty()
                && cx.dir.starts_with(workspace.root().join(&member.dir))
        });
        return Ok(if inside {
            Scope::Project
        } else {
            Scope::Every(workspace)
        });
    }
    let workspace = found.ok_or_else(|| no_workspace(cx.dir))??;
    let mut named = Vec::with_capacity(specs.len());
    for spec in specs {
        match workspace.find(spec) {
            Ok(member) => named.push(member.dir.clone()),
            Err(Unmatched::Unknown) => return Err(unknown_member(spec, &workspace)),
            Err(Unmatched::Ambiguous(members)) => return Err(ambiguous_member(spec, &members)),
        }
    }
    let dirs = (workspace.members().iter())
        .filter(|member| named.contains(&member.dir))
        .map(|member| member.dir.clone())
        .collect();
    Ok(Scope::Named(workspace, dirs))
}

/// Runs `command` in the one project `spec` names, or the project
/// around the directory without it; what it gives, or `failed` once what
/// was wrong is shown, as when the directory is in a workspace but in
/// none of its members, which `verb` then needs one of.
pub fn in_one<H: Http, T>(
    cx: &Context<'_, H>,
    console: &mut dyn Console,
    spec: Option<&str>,
    verb: &str,
    failed: T,
    command: impl FnOnce(&Context<'_, H>, &mut dyn Console) -> T,
) -> T {
    let specs: Vec<String> = spec.iter().map(|s| (*s).to_owned()).collect();
    let dir = match find(cx, &specs) {
        Ok(Scope::Project) => cx.dir.to_owned(),
        Ok(Scope::Named(workspace, dirs)) => workspace.root().join(&dirs[0]),
        Ok(Scope::Every(workspace)) => {
            console::report(console, &which_member(verb, &workspace), Compat::LATEST);
            return failed;
        }
        Err(report) => {
            console::report(console, &report, Compat::LATEST);
            return failed;
        }
    };
    command(&Context { dir: &dir, ..*cx }, console)
}

/// `nomi build` or `nomi check` as `make` asks, of the projects `specs`
/// names: with paths, those modules of the project around the directory;
/// without, the entries of each project's `nomi.toml` and member
/// packages' exposed modules, or, in a project without entries, what
/// `elm make` makes without paths. Whether nothing was wrong.
pub fn make(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    specs: &[String],
    make: &Make,
) -> bool {
    if !make.paths.is_empty() {
        return make::run(cx, console, make);
    }
    let scope = match find(cx, specs) {
        Ok(scope) => scope,
        Err(report) => return failed(console, &report),
    };
    let (workspace, dirs) = match scope {
        Scope::Project => {
            let snapshot = cx.vfs.snapshot();
            match (making(make), entries(&snapshot, cx)) {
                (Ok(making), Some(Ok(workspace))) => {
                    return built(cx, console, &workspace, &[PathBuf::new()], making);
                }
                (Ok(_), Some(Err(report))) => return failed(console, &report),
                (Err(_), _) | (Ok(_), None) => return make::run(cx, console, make),
            }
        }
        Scope::Named(workspace, dirs) => (workspace, dirs),
        Scope::Every(workspace) => {
            let dirs = (workspace.members().iter())
                .map(|m| m.dir.clone())
                .collect::<Vec<_>>();
            (workspace, dirs)
        }
    };
    let making = match making(make) {
        Ok(making) => making,
        Err(report) => return failed(console, &report),
    };
    match workspace::resolved(cx, workspace) {
        Ok(workspace) => built(cx, console, &workspace, &dirs, making),
        Err(report) => failed(console, &report),
    }
}

/// The project around the directory as a workspace of its one member,
/// where its `nomi.toml` has entries.
fn entries(snapshot: &Snapshot, cx: &Context<'_, impl Http>) -> Option<Result<Workspace, Report>> {
    let root = Root::find(cx.vfs, cx.dir)?;
    workspace::alone(snapshot, root.dir())
}

/// What building entries in place of paths makes for `make`, which must
/// ask for nothing only paths have: a check where nothing is written,
/// else code of the kind it asks for.
fn making(make: &Make) -> Result<Making, Report> {
    let kind = make.kind.map_err(|_| make::cannot_optimize_and_debug())?;
    let plain = make.docs.is_none() && make.ffi.is_none() && make.style == Style::Terminal;
    match &make.output {
        None if plain && make.platform.is_none() => Ok(Making::Code(kind)),
        Some(Output::DevNull) if plain => Ok(Making::Check),
        _ => Err(flags_need_paths()),
    }
}

/// [`workspace::build`]: whether every member built.
fn built(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    workspace: &Workspace,
    dirs: &[PathBuf],
    making: Making,
) -> bool {
    let outcomes = workspace::build(cx, console, workspace, dirs, making);
    (outcomes.iter()).all(|(_, outcome)| matches!(outcome, Outcome::Built(_)))
}

fn failed(console: &mut dyn Console, report: &Report) -> bool {
    console::report(console, report, Compat::LATEST);
    false
}

/// The members, each by its directory, and a package's by its name too,
/// as a sentence lists them.
fn listed<'m>(members: impl IntoIterator<Item = &'m Member>) -> String {
    let names: Vec<String> = (members.into_iter())
        .map(|member| {
            let dir = if member.dir.as_os_str().is_empty() {
                ".".to_owned()
            } else {
                member.dir.display().to_string()
            };
            match &member.outline {
                Outline::Pkg(pkg) => format!("{dir} ({})", pkg.name),
                Outline::App(_) => dir,
            }
        })
        .collect();
    match names.split_last() {
        None => "none".to_owned(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

fn no_workspace(dir: &Path) -> Report {
    one_line(
        "NO WORKSPACE",
        &format!(
            "-p names a member of a workspace, but no nomi.toml with a [workspace] holds {}.",
            dir.display()
        ),
    )
}

fn unknown_member(spec: &str, workspace: &Workspace) -> Report {
    one_line(
        "UNKNOWN MEMBER",
        &format!(
            "No member of the workspace is {spec}. Name one by its directory or its package's \
             name: {}.",
            listed(workspace.members())
        ),
    )
}

fn ambiguous_member(spec: &str, members: &[&Member]) -> Report {
    one_line(
        "AMBIGUOUS MEMBER",
        &format!(
            "{spec} could be any of the members {}. Name one by its directory.",
            listed(members.iter().copied())
        ),
    )
}

fn which_member(verb: &str, workspace: &Workspace) -> Report {
    one_line(
        "WHICH MEMBER?",
        &format!(
            "nomi {verb} works on one project, and here there is the whole workspace. Name a \
             member with -p, or run it in the member's directory: {}.",
            listed(workspace.members())
        ),
    )
}

fn flags_need_paths() -> Report {
    one_line(
        "FLAGS NEED MODULES",
        "--output, --platform, --ffi, --docs and --report apply to the modules named on the \
         command line. Without them, nomi builds each entry of nomi.toml as the entry says.",
    )
}
