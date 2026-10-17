//! `nomi rm`, which upstream has no command for: the plan for taking a
//! package out of the project, shown and, once the user agrees, written
//! to `elm.json`. Its messages follow `install`'s. In a workspace, the
//! member packages are taken from it, and the workspace is checked out
//! once the member's `elm.json` changes.

use nomi_base::diagnostic::doc::{Color, reflow, text, vcat};
use nomi_base::{Compat, Report};
use nomi_project::report::elm_json_path;
use nomi_project::{Name, Outline};
use nomi_registry::workspace::Members;
use nomi_registry::{Http, Registry, RemoveProblem, SolverProblem, remove, report};

use crate::console::{self, Console, Personality};
use crate::install::{parse_name, plan_question, verify_member};
use crate::project::{Project, elm_json};
use crate::report::one_line;
use crate::update::unavailable;
use crate::{Context, env, workspace};

/// Removes `package` from the project around the directory, once the user
/// agrees. Whether it succeeded; a decline is a success.
pub fn run(cx: &Context<'_, impl Http + Sync>, console: &mut dyn Console, package: &str) -> bool {
    let package = match parse_name(package) {
        Ok(package) => package,
        Err(problem) => return console::reported(console, Err(problem), Compat::LATEST),
    };
    env::with_registry(
        cx,
        console,
        "to remove a package from",
        "I need the list of published packages to figure out what to remove",
        |console, project, registry| remove(cx, console, &package, project, registry),
    )
}

fn remove<H: Http + Sync>(
    cx: &Context<'_, H>,
    console: &mut dyn Console,
    package: &Name,
    project: &Project,
    registry: &mut Registry<'_, H>,
) -> Result<bool, Report> {
    let workspace = workspace::discover(&cx.vfs.snapshot(), project.root.dir()).transpose()?;
    let plan = match &workspace {
        None => remove::plan(registry, package, &project.outline),
        Some(workspace) => remove::plan(
            &mut Members::new(workspace, registry),
            package,
            &project.outline,
        ),
    };
    let (question, new) = match plan {
        Ok(remove::Plan::App { changes, outline }) => {
            (plan_question(&changes), Outline::App(outline))
        }
        Ok(remove::Plan::Pkg { changes, outline }) => {
            (plan_question(&changes), Outline::Pkg(outline))
        }
        Err(problem) => return Err(remove_problem(package, problem)),
    };
    console::confirm(
        console,
        &question,
        project.compat,
        Personality::Nomi,
        || {
            if workspace.is_none() {
                return project.write(cx.vfs, new.write());
            }
            let src = elm_json(&cx.vfs.snapshot(), &project.root)?;
            project.write(cx.vfs, new.write())?;
            let store = registry.store();
            let verified = verify_member(cx, registry, store, project.compat, &project.root, &new);
            if verified.is_err() {
                project.write(cx.vfs, &src)?;
            }
            verified
        },
    )
}

fn remove_problem(package: &Name, problem: RemoveProblem<SolverProblem>) -> Report {
    match problem {
        RemoveProblem::NotADependency => one_line(
            "NOT A DEPENDENCY",
            &format!("Your elm.json does not depend on {package}, so there is nothing to remove."),
        ),
        RemoveProblem::Indirect(dependents) => Report::new(
            "INDIRECT DEPENDENCY",
            elm_json_path(),
            &format!(
                "Your elm.json has {package} only because other dependencies need it, so I \
                 cannot remove it on its own. These are the ones that need it:"
            ),
            [
                vcat(dependents.iter().map(|name| text(name.to_string())))
                    .color(Color::DullYellow)
                    .indent(4),
                reflow("It goes once nothing needs it any more."),
            ],
        ),
        RemoveProblem::Unavailable(name, version) => unavailable(&name, version),
        RemoveProblem::Solver(problem) => report::solver_problem(&problem),
    }
}
