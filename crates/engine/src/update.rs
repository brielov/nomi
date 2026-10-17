//! `nomi update` and `nomi outdated`, which upstream has no commands for:
//! the plan for bringing the dependencies up to date, shown and, once the
//! user agrees, written to `elm.json`, and the dependencies with newer
//! releases. Their messages follow `install`'s.

use std::fmt::Display;
use std::path::PathBuf;

use nomi_base::{Compat, Report};
use nomi_project::outline::AppOutline;
use nomi_project::workspace::Workspace;
use nomi_project::{Name, Outline, Version};
use nomi_registry::update::DepKind;
use nomi_registry::workspace::{self as registry_workspace, Members};
use nomi_registry::{
    Http, Outdated, Registry, SolverProblem, Store, UpdateProblem, report, update,
};

use crate::console::{self, Console, Personality};
use crate::install::{parse_name, plan_question, verify_member};
use crate::packages::{self, Resolve};
use crate::paths::Root;
use crate::project::{Project, elm_json, write_elm_json};
use crate::report::one_line;
use crate::scope::{self, Scope};
use crate::{Context, env, workspace};

/// Why `update` and `outdated` need the package list.
const NEWER: &str = "I need the list of published packages to find newer releases";

/// What `update` says of packages, whose ranges it leaves.
const RANGES_STAY: &str = "Packages keep ranges in elm.json, and builds take the newest releases \
                           they allow, so there is nothing to update!\n";

/// Updates the dependencies of the projects `specs` names, or only
/// `package` and those that need it, once the user agrees: the project
/// around the directory, or members of the workspace around it, whose
/// applications move together and keep agreeing with the others. Whether
/// it succeeded; a decline is a success.
pub fn run(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    package: Option<&str>,
    specs: &[String],
) -> bool {
    let package = match package.map(parse_name).transpose() {
        Ok(package) => package,
        Err(problem) => return console::reported(console, Err(problem), Compat::LATEST),
    };
    let members = match scope::find(cx, specs) {
        Ok(Scope::Project) => Root::find(cx.vfs, cx.dir).and_then(|root| {
            let workspace = workspace::discover(&cx.vfs.snapshot(), root.dir())?;
            Some(workspace.map(|workspace| {
                let dir = workspace::member_dir(&workspace, root.dir());
                (workspace, vec![dir])
            }))
        }),
        Ok(Scope::Named(workspace, dirs)) => Some(Ok((workspace, dirs))),
        Ok(Scope::Every(workspace)) => {
            let dirs = (workspace.members().iter())
                .map(|m| m.dir.clone())
                .collect();
            Some(Ok((workspace, dirs)))
        }
        Err(report) => Some(Err(report)),
    };
    match members {
        None => alone(cx, console, package.as_ref()),
        Some(Ok((workspace, dirs))) => together(cx, console, package.as_ref(), &workspace, &dirs),
        Some(Err(report)) => console::reported(console, Err(report), Compat::LATEST),
    }
}

/// [`run`] in a project in no workspace.
fn alone(cx: &Context<'_, impl Http>, console: &mut dyn Console, package: Option<&Name>) -> bool {
    env::with_registry(
        cx,
        console,
        "to update",
        NEWER,
        |console, project, registry| update(cx, console, package, project, registry),
    )
}

/// [`run`] for the members of `workspace` in `dirs`: their applications
/// updated as one, holding what the others pin.
fn together(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    package: Option<&Name>,
    workspace: &Workspace,
    dirs: &[PathBuf],
) -> bool {
    let apps: Vec<(PathBuf, &AppOutline)> = (workspace.members().iter())
        .filter(|member| dirs.contains(&member.dir))
        .filter_map(|member| match &member.outline {
            Outline::App(app) => Some((member.dir.clone(), app)),
            Outline::Pkg(_) => None,
        })
        .collect();
    let compat = (apps.first())
        .and_then(|(_, app)| (Compat::ALL.into_iter()).find(|&c| Version::compiler(c) == app.elm))
        .unwrap_or(Compat::LATEST);
    let plan = |console: &mut dyn Console| -> Result<bool, Report> {
        let Some(((dir, app), rest)) = apps.split_first() else {
            console.out(RANGES_STAY);
            return Ok(true);
        };
        registry_workspace::agree(workspace).map_err(|problem| {
            workspace::resolution_problem(&problem, compat, Personality::Nomi)
        })?;
        let store = packages::store_for(cx.cache, Resolve::Nomi)?;
        let mut registry = env::registry(cx.cache, &store, cx.http, compat, NEWER)?;
        let mut held = (Members::new(workspace, &mut registry))
            .holding_all_but(workspace, dirs)
            .within_ranges(workspace);
        let together = update::plan_together(&mut held, compat, package, (dir, app), rest)
            .map_err(|problem| update_problem(package, problem))?;
        if together.changes.is_empty() {
            console.out("Everything is up to date!\n");
            return Ok(true);
        }
        let question = plan_question(&together.changes);
        console::confirm(console, &question, compat, Personality::Nomi, || {
            let mut written = Vec::with_capacity(together.apps.len());
            let result = write_and_verify(
                cx,
                &mut registry,
                &store,
                compat,
                workspace,
                &together,
                &mut written,
            );
            if result.is_err() {
                for (root, src) in written.iter().rev() {
                    write_elm_json(cx.vfs, root, src)?;
                }
            }
            result
        })
    };
    let result = plan(console);
    console::reported(console, result, compat)
}

/// Writes each application's `elm.json` as `together` has it, keeping in
/// `written` each one's directory and what it held, then checks the
/// workspace's dependencies out together, as `add` checks a member's,
/// since the member packages' ranges, their tests' among them, may rule
/// out what the applications moved to.
fn write_and_verify(
    cx: &Context<'_, impl Http + Sync>,
    registry: &mut Registry<'_, impl Http + Sync>,
    store: &Store,
    compat: Compat,
    workspace: &Workspace,
    together: &update::Together,
    written: &mut Vec<(Root, Vec<u8>)>,
) -> Result<(), Report> {
    for (dir, app) in &together.apps {
        let root = Root::new(workspace.root().join(dir));
        let src = elm_json(&cx.vfs.snapshot(), &root)?;
        write_elm_json(cx.vfs, &root, Outline::App(app.clone()).write())?;
        written.push((root, src));
    }
    for ((_, app), (root, _)) in together.apps.iter().zip(written.iter()) {
        verify_member(
            cx,
            registry,
            store,
            compat,
            root,
            &Outline::App(app.clone()),
        )?;
    }
    Ok(())
}

fn update<H: Http>(
    cx: &Context<'_, H>,
    console: &mut dyn Console,
    package: Option<&Name>,
    project: &Project,
    registry: &mut Registry<'_, H>,
) -> Result<bool, Report> {
    let plan = update::plan(registry, project.compat, package, &project.outline)
        .map_err(|problem| update_problem(package, problem))?;
    let (changes, outline) = match plan {
        update::Plan::Pkg => {
            console.out(RANGES_STAY);
            return Ok(true);
        }
        update::Plan::App { changes, .. } if changes.is_empty() => {
            console.out("Everything is up to date!\n");
            return Ok(true);
        }
        update::Plan::App { changes, outline } => (changes, Outline::App(outline)),
    };
    console::confirm(
        console,
        &plan_question(&changes),
        project.compat,
        Personality::Nomi,
        || project.write(cx.vfs, outline.write()),
    )
}

fn update_problem(package: Option<&Name>, problem: UpdateProblem<SolverProblem>) -> Report {
    match problem {
        UpdateProblem::NotADependency => one_line(
            "NOT A DEPENDENCY",
            &format!(
                "Your elm.json does not depend on {}, so there is nothing to update.",
                package.map_or_else(String::new, Name::to_string)
            ),
        ),
        UpdateProblem::Unavailable(name, version) => unavailable(&name, version),
        UpdateProblem::NoSolution => one_line(
            "CANNOT FIND COMPATIBLE VERSIONS",
            "I cannot find a set of packages that satisfies elm.json, even when I let each \
             direct dependency move up to its next major version.",
        ),
        UpdateProblem::Solver(problem) => report::solver_problem(&problem),
    }
}

/// That the `elm.json` of a release the plan needs couldn't be had.
pub(crate) fn unavailable(name: &Name, version: Version) -> Report {
    one_line(
        "PROBLEM LOADING PACKAGE",
        &format!(
            "I need the elm.json of {name} {version} to know what it depends on, but I could \
             not get it."
        ),
    )
}

/// Lists the dependencies of the project around the directory that have
/// newer releases. Whether it succeeded.
pub fn outdated(cx: &Context<'_, impl Http>, console: &mut dyn Console) -> bool {
    env::with_registry(
        cx,
        console,
        "to check",
        NEWER,
        |console, project, registry| {
            console.out(&match &project.outline {
                Outline::App(app) => table(&update::outdated_app(registry, app)),
                Outline::Pkg(pkg) => table(&update::outdated_pkg(registry, pkg)),
            });
            Ok(true)
        },
    )
}

/// The outdated dependencies in columns, or that there are none.
fn table<T: Display>(outdated: &[Outdated<T>]) -> String {
    if outdated.is_empty() {
        return "Everything is up to date!\n".to_owned();
    }
    let rows: Vec<[String; 4]> =
        std::iter::once(["Package", "Current", "Compatible", "Latest"].map(str::to_owned))
            .chain(outdated.iter().map(|o| {
                let name = match o.kind {
                    DepKind::Normal => o.name.to_string(),
                    DepKind::Test => format!("{} (test)", o.name),
                };
                let compatible = o
                    .compatible
                    .map_or_else(|| "-".to_owned(), |v| v.to_string());
                [
                    name,
                    o.current.to_string(),
                    compatible,
                    o.latest.to_string(),
                ]
            }))
            .collect();
    let widths: Vec<usize> = (0..4)
        .map(|i| {
            rows.iter()
                .map(|row| row[i].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut table = String::new();
    for row in &rows {
        let cells: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(cell, &width)| format!("{cell:width$}"))
            .collect();
        table.push_str(cells.join("  ").trim_end());
        table.push('\n');
    }
    table
}

#[cfg(test)]
mod tests {
    use nomi_project::Constraint;

    use super::*;

    #[test]
    fn outdated_dependencies_line_up() {
        let compat = Compat::LATEST;
        let name = |s: &str| s.parse::<Name>().unwrap();
        let v = |s: &str| s.parse::<Version>().unwrap();
        let app = [
            Outdated {
                name: name("elm/http"),
                current: v("1.0.0"),
                compatible: Some(v("1.1.0")),
                latest: v("2.0.0"),
                kind: DepKind::Normal,
            },
            Outdated {
                name: name("elm-explorations/test"),
                current: v("2.0.0"),
                compatible: None,
                latest: v("3.0.0"),
                kind: DepKind::Test,
            },
        ];
        assert_eq!(
            table(&app),
            "\
Package                       Current  Compatible  Latest
elm/http                      1.0.0    1.1.0       2.0.0
elm-explorations/test (test)  2.0.0    -           3.0.0
"
        );
        let pkg = [Outdated {
            name: name("elm/http"),
            current: Constraint::until_next_major(v("1.0.0"), compat),
            compatible: Some(v("1.1.0")),
            latest: v("2.0.0"),
            kind: DepKind::Normal,
        }];
        assert_eq!(
            table(&pkg),
            "\
Package   Current             Compatible  Latest
elm/http  1.0.0 <= v < 2.0.0  1.1.0       2.0.0
"
        );
        assert_eq!(table::<Version>(&[]), "Everything is up to date!\n");
    }
}
