//! `nomi rm`, which upstream has no command for: the plan for taking a
//! package out of the project, shown and, once the user agrees, written
//! to `elm.json`. Its messages follow `install`'s.

use nomi_base::diagnostic::doc::{Color, reflow, text, vcat};
use nomi_base::{Compat, Report};
use nomi_project::report::elm_json_path;
use nomi_project::{Name, Outline};
use nomi_registry::{Http, RemoveProblem, SolverProblem, Store, remove, report};

use crate::console::{self, Console, Personality};
use crate::install::{parse_name, plan_question};
use crate::project::Project;
use crate::report::one_line;
use crate::update::unavailable;
use crate::{Context, env};

/// Removes `package` from the project around the directory, once the user
/// agrees. Whether it succeeded; a decline is a success.
pub fn run(cx: &Context<'_, impl Http>, console: &mut dyn Console, package: &str) -> bool {
    let mut compat = Compat::LATEST;
    let result = remove(cx, console, package, &mut compat);
    console::reported(console, result, compat)
}

fn remove(
    cx: &Context<'_, impl Http>,
    console: &mut dyn Console,
    package: &str,
    compat: &mut Compat,
) -> Result<bool, Report> {
    let package = parse_name(package)?;
    let project = Project::load(cx.vfs, cx.dir, "to remove a package from")?;
    *compat = project.compat;
    let store = Store::new(cx.cache.packages());
    let mut registry = env::registry(
        cx.cache,
        &store,
        cx.http,
        *compat,
        "I need the list of published packages to figure out what to remove",
    )?;
    let (question, new) = match remove::plan(&mut registry, &package, &project.outline) {
        Ok(remove::Plan::App { changes, outline }) => {
            (plan_question(&changes), Outline::App(outline))
        }
        Ok(remove::Plan::Pkg { changes, outline }) => {
            (plan_question(&changes), Outline::Pkg(outline))
        }
        Err(problem) => return Err(remove_problem(&package, problem)),
    };
    console::confirm(console, &question, *compat, Personality::Nomi, || {
        project.write(cx.vfs, new.write())
    })
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
