//! `nomi update` and `nomi outdated`, which upstream has no commands for:
//! the plan for bringing the dependencies up to date, shown and, once the
//! user agrees, written to `elm.json`, and the dependencies with newer
//! releases. Their messages follow `install`'s.

use std::fmt::Display;

use nomi_base::{Compat, Report};
use nomi_project::{Name, Outline, Version};
use nomi_registry::update::DepKind;
use nomi_registry::{Http, Outdated, Registry, SolverProblem, UpdateProblem, report, update};

use crate::console::{self, Console, Personality};
use crate::install::{parse_name, plan_question};
use crate::project::Project;
use crate::report::one_line;
use crate::{Context, env};

/// Updates the dependencies of the project around the directory, or
/// only `package` and those that need it, once the user agrees. Whether
/// it succeeded; a decline is a success.
pub fn run(cx: &Context<'_, impl Http>, console: &mut dyn Console, package: Option<&str>) -> bool {
    let package = match package.map(parse_name).transpose() {
        Ok(package) => package,
        Err(problem) => return console::reported(console, Err(problem), Compat::LATEST),
    };
    env::with_registry(
        cx,
        console,
        "to update",
        "I need the list of published packages to find newer releases",
        |console, project, registry| update(cx, console, package.as_ref(), project, registry),
    )
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
            console.out(
                "Packages keep ranges in elm.json, and builds take the newest releases \
                 they allow, so there is nothing to update!\n",
            );
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
        "I need the list of published packages to find newer releases",
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
