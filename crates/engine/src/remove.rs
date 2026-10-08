//! `nomi rm`, which upstream has no command for: the plan for taking a
//! package out of the project, shown and, once the user agrees, written
//! to `elm.json`. Its messages follow `install`'s.

use std::path::Path;

use nomi_base::diagnostic::doc::{Color, reflow, text, vcat};
use nomi_base::{Compat, Report};
use nomi_project::{Name, Outline};
use nomi_registry::{Http, RemoveProblem, SolverProblem, Store, remove, report};

use crate::console::{self, Console, Personality};
use crate::install::{one_line, parse_name, plan_question};
use crate::paths::Cache;
use crate::project::Project;
use crate::{Vfs, env};

/// Removes `package` from the project around `dir`, once the user agrees.
/// Whether it succeeded; a decline is a success.
pub fn run(
    vfs: &Vfs,
    console: &mut dyn Console,
    dir: &Path,
    cache: &Cache,
    http: &impl Http,
    package: &str,
) -> bool {
    let mut compat = Compat::LATEST;
    match remove(vfs, console, dir, cache, http, package, &mut compat) {
        Ok(succeeded) => succeeded,
        Err(problem) => {
            console::report(console, &problem, compat);
            false
        }
    }
}

fn remove(
    vfs: &Vfs,
    console: &mut dyn Console,
    dir: &Path,
    cache: &Cache,
    http: &impl Http,
    package: &str,
    compat: &mut Compat,
) -> Result<bool, Report> {
    let package = parse_name(package)?;
    let project = Project::load(vfs, dir, "to remove a package from")?;
    *compat = project.compat;
    let store = Store::new(cache.packages());
    let mut registry = env::registry(
        cache,
        &store,
        http,
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
    match console::ask(console, &question, *compat) {
        Err(e) => Err(one_line("PROBLEM READING ANSWER", e.to_string())),
        Ok(None) => {
            console::no_answer(console, Personality::Nomi);
            Ok(false)
        }
        Ok(Some(false)) => {
            console.out("Okay, I did not change anything!\n");
            Ok(true)
        }
        Ok(Some(true)) => {
            project.write(vfs, &new)?;
            console.out("Success!\n");
            Ok(true)
        }
    }
}

fn remove_problem(package: &Name, problem: RemoveProblem<SolverProblem>) -> Report {
    match problem {
        RemoveProblem::NotADependency => one_line(
            "NOT A DEPENDENCY",
            format!("Your elm.json does not depend on {package}, so there is nothing to remove."),
        ),
        RemoveProblem::Indirect(dependents) => Report::new(
            "INDIRECT DEPENDENCY",
            Some("elm.json".to_owned()),
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
        RemoveProblem::Unavailable(name, version) => one_line(
            "PROBLEM LOADING PACKAGE",
            format!(
                "I need the elm.json of {name} {version} to know what it depends on, but I \
                 could not get it."
            ),
        ),
        RemoveProblem::Solver(problem) => report::solver_problem(&problem),
    }
}
