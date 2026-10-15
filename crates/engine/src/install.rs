//! `elm install` and `nomi add` (`Install.hs`, the same in every supported
//! release): the plan for adding a package to the project, shown and,
//! once the user agrees, written to `elm.json`, whose dependencies are
//! then downloaded.
//!
//! Upstream goes on to build the dependencies, which nomi's builds do
//! when they need them, and holds the project's lock while it works.

use std::collections::BTreeMap;
use std::fmt::Display;

use nomi_base::diagnostic::doc::{
    Color, Doc, fill_sep, hint, hsep, reflow, simple_note, text, vcat, words,
};
use nomi_base::{Compat, Report};
use nomi_project::report::{ElmCommand, elm_json_path};
use nomi_project::{Name, Outline};
use nomi_registry::{
    Change, Connection, DetailsProblem, Http, InstallProblem, Plan, Registry, SolverProblem, Store,
    details, install, report,
};

use crate::console::{self, Console, Personality};
use crate::paths::{Cache, Root};
use crate::project::{self, Project};
use crate::report::one_line;
use crate::{Context, Pool, env, packages};

/// Adds `package` to the project around the directory, once the user
/// agrees; with no package, says what `install` is for. Whether it
/// succeeded; a decline is a success.
pub fn run(
    cx: &Context<'_, impl Http + Sync>,
    console: &mut dyn Console,
    personality: Personality,
    package: Option<&str>,
) -> bool {
    let mut compat = Compat::LATEST;
    let mut add = |console: &mut dyn Console| -> Result<bool, Report> {
        // Upstream's argument parser rejects a bad name before anything
        // looks for the project.
        let package = package.map(parse_name).transpose()?;
        let Some(root) = Root::find(cx.vfs, cx.dir) else {
            return Err(no_outline(personality));
        };
        let Some(package) = &package else {
            return Err(no_args(personality, cx.cache));
        };
        let src = project::elm_json(cx.vfs, &root)?;
        // Upstream loads the registry before it reads the outline, so its
        // problems come first; reading it as the release it names comes
        // first here.
        let project = Project::decode(&root, &src, personality);
        if let Ok(project) = &project {
            compat = project.compat;
        }
        let store = packages::store_for(cx.cache, personality)?;
        let mut registry = env::registry(
            cx.cache,
            &store,
            cx.http,
            compat,
            "I need the list of published packages to figure out how to install things",
        )?;
        let project = project?;
        let old = &project.outline;
        project::check_source_dirs(root.dir(), old)?;
        let plan = install::plan(&mut registry, compat, package, old)
            .map_err(|problem| install_problem(&registry, package, old, problem))?;
        let (question, new) = match plan {
            Plan::AlreadyInstalled => {
                console.out("It is already installed!\n");
                return Ok(true);
            }
            Plan::PromoteIndirect(new) => (promote_indirect(), new),
            Plan::PromoteTest(new) => (promote_test(), new),
            Plan::App { changes, outline } => (plan_question(&changes), Outline::App(outline)),
            Plan::Pkg {
                changes, outline, ..
            } => (plan_question(&changes), Outline::Pkg(outline)),
        };
        console::confirm(console, &question, compat, personality, || {
            project.write(cx.vfs, new.write())?;
            let verified = verify(&mut registry, &store, cx.http, compat, personality, &new);
            if verified.is_err() {
                project.write(cx.vfs, &src)?;
            }
            verified
        })
    };
    let result = add(console);
    console::reported(console, result, compat)
}

/// `InstallNoOutline`.
fn no_outline(personality: Personality) -> Report {
    Report::new(
        "NEW PROJECT?",
        None,
        "Are you trying to start a new project? Try this command instead:",
        [
            text(personality.command(ElmCommand::Init))
                .color(Color::Green)
                .indent(4),
            reflow("It will help you get started!"),
        ],
    )
}

/// `InstallNoArgs`, naming nomi's package store where upstream names
/// `ELM_HOME`.
fn no_args(personality: Personality, cache: &Cache) -> Report {
    let install = personality.command(ElmCommand::Install);
    let make = personality.command(ElmCommand::Make);
    Report::new(
        "INSTALL WHAT?",
        None,
        "I am expecting commands like:",
        [
            vcat(
                ["elm/http", "elm/json", "elm/random"]
                    .map(|package| text(format!("{install} {package}"))),
            )
            .indent(4)
            .color(Color::Green),
            hint(
                words(
                    "In JavaScript folks run `npm install` to start projects. \"Gotta \
                     download everything!\" But why download packages again and again? \
                     Instead, Elm caches packages in",
                )
                .chain([text(cache.packages().display().to_string()).color(Color::DullYellow)])
                .chain(words(
                    "so each one is downloaded and built ONCE on your machine. Elm projects \
                     check that cache before trying the internet. This reduces build times, \
                     reduces server costs, and makes it easier to work offline. As a result",
                ))
                .chain([text(install).color(Color::DullCyan)])
                .chain(words(
                    "is only for adding dependencies to elm.json, whereas",
                ))
                .chain([text(make.clone()).color(Color::DullCyan)])
                .chain(words(
                    "is in charge of gathering dependencies and building everything. So \
                     maybe try",
                ))
                .chain([text(make).color(Color::Green), text("instead?")]),
            ),
        ],
    )
}

fn install_problem<H: Http>(
    registry: &Registry<'_, H>,
    package: &Name,
    outline: &Outline,
    problem: InstallProblem<SolverProblem>,
) -> Report {
    match problem {
        InstallProblem::UnknownPackage => {
            let nearby = registry
                .index
                .versions_or_nearby(package)
                .err()
                .unwrap_or_default();
            unknown_package(package, &nearby, registry.connection)
        }
        InstallProblem::NoSolution => no_solution(package, outline, registry.connection),
        InstallProblem::Solver(problem) => report::solver_problem(&problem),
    }
}

/// `Details.verifyInstall`, short of building: the new outline's
/// dependencies check out and are downloaded.
fn verify(
    registry: &mut Registry<'_, impl Http + Sync>,
    store: &Store,
    http: &(impl Http + Sync),
    compat: Compat,
    personality: Personality,
    outline: &Outline,
) -> Result<(), Report> {
    let solution = details::verify(registry, compat, outline)
        .map_err(|problem| details_problem(&problem, compat, personality))?;
    let releases: Vec<_> = (solution.into_iter())
        .map(|(name, details)| (name, details.version))
        .collect();
    let fetched = packages::fetch(Pool::global(), store, http, compat, &releases);
    // Upstream reports the first of the releases it couldn't download.
    for ((name, version), fetched) in releases.iter().zip(fetched) {
        fetched.map_err(|problem| report::package_problem(name, *version, &problem))?;
    }
    Ok(())
}

/// `toDetailsReport`, in `personality`'s words.
pub(crate) fn details_problem(
    problem: &DetailsProblem<SolverProblem>,
    compat: Compat,
    personality: Personality,
) -> Report {
    let reactor = (personality == Personality::Elm).then_some("elm reactor");
    report::details_problem(problem, compat, &|c| personality.command(c), reactor)
}

/// The package argument. Upstream's argument parser reports a bad one
/// with examples, which come with the rest of its usage errors.
pub(crate) fn parse_name(arg: &str) -> Result<Name, Report> {
    arg.parse().map_err(|_| {
        one_line(
            "BAD PACKAGE NAME",
            &format!("{arg} is not a package name, which looks like elm/http."),
        )
    })
}

fn promote_indirect() -> Doc {
    vcat([
        fill_sep(
            words("I found it in your elm.json file, but in the")
                .chain([text("\"indirect\"").color(Color::DullYellow)])
                .chain(words("dependencies.")),
        ),
        fill_sep(
            words("Should I move it into")
                .chain([text("\"direct\"").color(Color::Green)])
                .chain(words("dependencies for more general use?"))
                .chain([text("[Y/n]: ")]),
        ),
    ])
}

fn promote_test() -> Doc {
    vcat([
        fill_sep(
            words("I found it in your elm.json file, but in the")
                .chain([text("\"test-dependencies\"").color(Color::DullYellow)])
                .chain(words("field.")),
        ),
        fill_sep(
            words("Should I move it into")
                .chain([text("\"dependencies\"").color(Color::Green)])
                .chain(words("for more general use?"))
                .chain([text("[Y/n]: ")]),
        ),
    ])
}

/// The `Changes` question: the plan, by kind of change.
pub(crate) fn plan_question<T: Display>(changes: &BTreeMap<Name, Change<T>>) -> Doc {
    let width = |s: &dyn Display| s.to_string().chars().count();
    let (mut name_w, mut left_w, mut right_w) = (0, 0, 0);
    for (name, change) in changes {
        name_w = name_w.max(width(name));
        match change {
            Change::Insert(v) | Change::Remove(v) => left_w = left_w.max(width(v)),
            Change::Change { old, new } => {
                left_w = left_w.max(width(old));
                right_w = right_w.max(width(new));
            }
        }
    }
    let pad = |w: usize, v: &T| {
        let v = v.to_string();
        text(format!(
            "{}{v}",
            " ".repeat(w.saturating_sub(v.chars().count()))
        ))
    };
    let name = |name: &Name| text(name.to_string()).fill(name_w + 3);
    let (mut inserts, mut changed, mut removes) = (Vec::new(), Vec::new(), Vec::new());
    for (n, change) in changes {
        match change {
            Change::Insert(v) => inserts.push(name(n).space(pad(left_w, v))),
            Change::Change { old, new } => changed.push(hsep([
                name(n),
                pad(left_w, old),
                text("=>"),
                pad(right_w, new),
            ])),
            Change::Remove(v) => removes.push(name(n).space(pad(left_w, v))),
        }
    }
    let section = |title: &str, entries: Vec<Doc>| {
        if entries.is_empty() {
            vec![]
        } else {
            vec![Doc::empty(), text(title), vcat(entries).indent(2)]
        }
    };
    let docs = [
        section("Add:", inserts),
        section("Change:", changed),
        section("Remove:", removes),
    ]
    .concat();
    vcat([
        text("Here is my plan:"),
        vcat(docs).indent(2),
        Doc::empty(),
        text("Would you like me to update your elm.json accordingly? [Y/n]: "),
    ])
}

/// `InstallUnknownPackageOnline` and `InstallUnknownPackageOffline`.
fn unknown_package(package: &Name, nearby: &[&Name], connection: Connection) -> Report {
    let suggestions = vcat(nearby.iter().map(|name| text(name.to_string())))
        .color(Color::DullYellow)
        .indent(4);
    let start = fill_sep(
        words("I cannot find a package named").chain([text(package.to_string())
            .color(Color::Red)
            .append(text("."))]),
    );
    let explanation = match connection {
        Connection::Online => vec![
            reflow(
                "I looked through https://package.elm-lang.org for packages with similar names \
                 and found these:",
            ),
            suggestions,
        ],
        Connection::Offline => vec![
            reflow(
                "I could not connect to https://package.elm-lang.org though, so new packages may \
                 have been published since I last updated my local cache of package names.",
            ),
            reflow("Looking through the locally cached names, the closest ones are:"),
            suggestions,
        ],
    };
    Report::doc(
        "UNKNOWN PACKAGE",
        None,
        start,
        explanation
            .into_iter()
            .chain([reflow("Maybe you want one of these instead?")]),
    )
}

/// `InstallNo{Online,Offline}{App,Pkg}Solution`.
fn no_solution(package: &Name, outline: &Outline, connection: Connection) -> Report {
    let existing = match outline {
        Outline::App(_) => "dependencies",
        Outline::Pkg(_) => "constraints",
    };
    let start = format!(
        "I cannot find a version of {package} that is compatible with your existing {existing}."
    );
    let path = elm_json_path();
    match (connection, outline) {
        (Connection::Offline, _) => Report::new(
            "CANNOT FIND COMPATIBLE VERSION LOCALLY",
            path,
            &start,
            [
                reflow(
                    "I was not able to connect to https://package.elm-lang.org/ though, so I was \
                     only able to look through packages that you have downloaded in the past.",
                ),
                reflow("Try again later when you have internet!"),
            ],
        ),
        (Connection::Online, Outline::App(_)) => Report::new(
            "CANNOT FIND COMPATIBLE VERSION",
            path,
            &start,
            [
                reflow(
                    "I checked all the published versions. When that failed, I tried to find any \
                     compatible combination of these packages, even if it meant changing all your \
                     existing dependencies! That did not work either!",
                ),
                reflow(
                    "This is most likely to happen when a package is not upgraded yet. Maybe a new \
                     version of Elm came out recently? Maybe a common package was changed \
                     recently? Maybe a better package came along, so there was no need to upgrade \
                     this one? Try asking around https://elm-lang.org/community to learn what \
                     might be going on with this package.",
                ),
                simple_note(
                    "Whatever the case, please be kind to the relevant package authors! Having \
                     friendly interactions with users is great motivation, and conversely, getting \
                     berated by strangers on the internet sucks your soul dry. Furthermore, \
                     package authors are humans with families, friends, jobs, vacations, \
                     responsibilities, goals, etc. They face obstacles outside of their technical \
                     work you will never know about, so please assume the best and try to be \
                     patient and supportive!",
                ),
            ],
        ),
        (Connection::Online, Outline::Pkg(_)) => Report::new(
            "CANNOT FIND COMPATIBLE VERSION",
            path,
            &start,
            [
                reflow(
                    "With applications, I try to broaden the constraints to see if anything works, \
                     but messing with package constraints is much more delicate business. E.g. \
                     making your constraints stricter may make it harder for applications to find \
                     compatible dependencies. So fixing something here may break it for a lot of \
                     other people!",
                ),
                reflow(
                    "So I recommend making an application with the same dependencies as your \
                     package. See if there is a solution at all. From there it may be easier to \
                     figure out how to proceed in a way that will disrupt your users as little as \
                     possible. And the solution may be to help other package authors to get their \
                     packages updated, or to drop a dependency entirely.",
                ),
            ],
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use nomi_registry::HttpError;

    use super::*;
    use crate::Vfs;
    use crate::console::Captured;

    struct Unreachable;

    impl Http for Unreachable {
        fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
            panic!("an unexpected GET {url}")
        }
        fn post(&self, url: &str) -> Result<Vec<u8>, HttpError> {
            panic!("an unexpected POST {url}")
        }
    }

    /// The transcript `install_no_args` with nomi's store where elm's
    /// `ELM_HOME` is, which wraps the hint differently.
    #[test]
    fn install_what_names_the_package_store() {
        let project = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("elm.json"), "{}").unwrap();
        let mut console = Captured::default();
        let cache = Cache::new(PathBuf::from("/cache"));
        let cx = Context {
            vfs: &Vfs::new(),
            dir: project.path(),
            cache: &cache,
            http: &Unreachable,
        };
        let succeeded = run(&cx, &mut console, Personality::Elm, None);
        assert!(!succeeded);
        assert_eq!(console.out, "");
        // The same length on Windows, where the separator differs.
        let store = cache.packages().display().to_string();
        assert_eq!(
            console.err,
            "\
-- INSTALL WHAT? ---------------------------------------------------------------

I am expecting commands like:

    elm install elm/http
    elm install elm/json
    elm install elm/random

Hint: In JavaScript folks run `npm install` to start projects. \"Gotta download
everything!\" But why download packages again and again? Instead, Elm caches
packages in /cache/packages so each one is downloaded and built ONCE on your
machine. Elm projects check that cache before trying the internet. This reduces
build times, reduces server costs, and makes it easier to work offline. As a
result elm install is only for adding dependencies to elm.json, whereas elm make
is in charge of gathering dependencies and building everything. So maybe try
elm make instead?

"
            .replace("/cache/packages", &store)
        );
    }
}
