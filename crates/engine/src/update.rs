//! `nomi update` and `nomi outdated`, which upstream has no commands for:
//! the plan for bringing the dependencies up to date, shown and, once the
//! user agrees, written to `elm.json`, and the dependencies with newer
//! releases. Their messages follow `install`'s.

use std::fmt::Display;
use std::path::Path;

use nomi_base::{Compat, Report};
use nomi_project::{Name, Outline};
use nomi_registry::{Http, Outdated, SolverProblem, Store, UpdateProblem, report, update};

use crate::console::{self, Console, Personality};
use crate::install::{one_line, parse_name, plan_question};
use crate::paths::Cache;
use crate::project::Project;
use crate::{Vfs, env};

/// Updates the dependencies of the project around `dir`, or only
/// `package` and those that need it, once the user agrees. Whether it
/// succeeded; a decline is a success.
pub fn run(
    vfs: &Vfs,
    console: &mut dyn Console,
    dir: &Path,
    cache: &Cache,
    http: &impl Http,
    package: Option<&str>,
) -> bool {
    let mut compat = Compat::LATEST;
    match update(vfs, console, dir, cache, http, package, &mut compat) {
        Ok(succeeded) => succeeded,
        Err(problem) => {
            console::report(console, &problem, compat);
            false
        }
    }
}

fn update(
    vfs: &Vfs,
    console: &mut dyn Console,
    dir: &Path,
    cache: &Cache,
    http: &impl Http,
    package: Option<&str>,
    compat: &mut Compat,
) -> Result<bool, Report> {
    let package = package.map(parse_name).transpose()?;
    let project = Project::load(vfs, dir, "to update")?;
    *compat = project.compat;
    let store = Store::new(cache.packages());
    let mut registry = env::registry(
        cache,
        &store,
        http,
        *compat,
        "I need the list of published packages to find newer releases",
    )?;
    let plan = update::plan(&mut registry, *compat, package.as_ref(), &project.outline)
        .map_err(|problem| update_problem(package.as_ref(), problem))?;
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
    match console::ask(console, &plan_question(&changes), *compat) {
        Err(e) => Err(one_line("PROBLEM READING ANSWER", &e.to_string())),
        Ok(None) => {
            console::no_answer(console, Personality::Nomi);
            Ok(false)
        }
        Ok(Some(false)) => {
            console.out("Okay, I did not change anything!\n");
            Ok(true)
        }
        Ok(Some(true)) => {
            project.write(vfs, &outline)?;
            console.out("Success!\n");
            Ok(true)
        }
    }
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
        UpdateProblem::Unavailable(name, version) => one_line(
            "PROBLEM LOADING PACKAGE",
            &format!(
                "I need the elm.json of {name} {version} to know what it depends on, but I \
                 could not get it."
            ),
        ),
        UpdateProblem::NoSolution => one_line(
            "CANNOT FIND COMPATIBLE VERSIONS",
            "I cannot find a set of packages that satisfies elm.json, even when I let each \
             direct dependency move up to its next major version.",
        ),
        UpdateProblem::Solver(problem) => report::solver_problem(&problem),
    }
}

/// Lists the dependencies of the project around `dir` that have newer
/// releases. Whether it succeeded.
pub fn outdated(
    vfs: &Vfs,
    console: &mut dyn Console,
    dir: &Path,
    cache: &Cache,
    http: &impl Http,
) -> bool {
    let mut compat = Compat::LATEST;
    let result = (|| {
        let project = Project::load(vfs, dir, "to check")?;
        compat = project.compat;
        let store = Store::new(cache.packages());
        let registry = env::registry(
            cache,
            &store,
            http,
            compat,
            "I need the list of published packages to find newer releases",
        )?;
        Ok::<_, Report>(match &project.outline {
            Outline::App(app) => table(&update::outdated_app(&registry, app)),
            Outline::Pkg(pkg) => table(&update::outdated_pkg(&registry, &pkg.deps, &pkg.test_deps)),
        })
    })();
    match result {
        Ok(table) => {
            console.out(&table);
            true
        }
        Err(problem) => {
            console::report(console, &problem, compat);
            false
        }
    }
}

/// The outdated dependencies in columns, or that there are none.
fn table<T: Display>(outdated: &[Outdated<T>]) -> String {
    if outdated.is_empty() {
        return "Everything is up to date!\n".to_owned();
    }
    let rows: Vec<[String; 4]> =
        std::iter::once(["Package", "Current", "Compatible", "Latest"].map(str::to_owned))
            .chain(outdated.iter().map(|o| {
                let name = if o.test {
                    format!("{} (test)", o.name)
                } else {
                    o.name.to_string()
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
    use nomi_project::json::Cursor;
    use nomi_project::{Constraint, Version};

    use super::*;

    #[test]
    fn outdated_dependencies_line_up() {
        let compat = Compat::LATEST;
        let name = |s: &str| Name::parse(s.as_bytes(), Cursor::start(compat), compat).unwrap();
        let v = |s: &str| Version::parse(s.as_bytes(), Cursor::start(compat), compat).unwrap();
        let app = [
            Outdated {
                name: name("elm/http"),
                current: v("1.0.0"),
                compatible: Some(v("1.1.0")),
                latest: v("2.0.0"),
                test: false,
            },
            Outdated {
                name: name("elm-explorations/test"),
                current: v("2.0.0"),
                compatible: None,
                latest: v("3.0.0"),
                test: true,
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
            test: false,
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
