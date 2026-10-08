//! `elm install` and `nomi add` (`Install.hs`, the same in every supported
//! release): the plan for adding a package to the project, shown and,
//! once the user agrees, written to `elm.json`, whose dependencies are
//! then downloaded.
//!
//! Upstream goes on to build the dependencies, which nomi's builds do
//! when they need them, and holds the project's lock while it works.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::path::Path;

use nomi_base::diagnostic::doc::{Color, Doc, fill_sep, hint, reflow, simple_note, text, vcat};
use nomi_base::{Compat, Report};
use nomi_project::json::Cursor;
use nomi_project::{Name, Outline};
use nomi_registry::{
    Change, Connection, DetailsProblem, Http, InstallProblem, PackageProblem, Plan, Registry,
    SolverProblem, Store, details, install, report,
};

use crate::console::{self, Console, Personality};
use crate::paths::{Cache, Root};
use crate::{Pool, Vfs, env, packages};

/// The path reports about the outline name.
const ELM_JSON: &str = "elm.json";

/// Adds `package` to the project around `dir`, once the user agrees; with
/// no package, says what `install` is for. Whether it succeeded; a
/// decline is a success.
pub fn run(
    vfs: &Vfs,
    console: &mut dyn Console,
    personality: Personality,
    dir: &Path,
    cache: &Cache,
    http: &(impl Http + Sync),
    package: Option<&str>,
) -> bool {
    let mut install = Install {
        vfs,
        console,
        personality,
        cache,
        compat: Compat::LATEST,
    };
    match install.run(dir, http, package) {
        Ok(succeeded) => succeeded,
        Err(problem) => {
            console::report(install.console, &problem, install.compat);
            false
        }
    }
}

struct Install<'a> {
    vfs: &'a Vfs,
    console: &'a mut dyn Console,
    personality: Personality,
    cache: &'a Cache,
    /// The release the project is built as, once its outline is read;
    /// until then the newest.
    compat: Compat,
}

impl Install<'_> {
    fn run(
        &mut self,
        dir: &Path,
        http: &(impl Http + Sync),
        package: Option<&str>,
    ) -> Result<bool, Report> {
        // Upstream's argument parser rejects a bad name before anything
        // looks for the project.
        let package = package.map(parse_name).transpose()?;
        let Some(root) = Root::find(self.vfs, dir) else {
            return Err(self.no_outline());
        };
        let Some(package) = &package else {
            return Err(self.no_args());
        };
        let path = root.elm_json();
        let src = self.vfs.read_bytes(&path).map_err(|e| {
            one_line(
                "PROBLEM READING ELM.JSON",
                format!("I could not read elm.json: {e}"),
            )
        })?;
        // Upstream loads the registry before it reads the outline, so its
        // problems come first; reading it as the release it names comes
        // first here.
        let detected = Outline::detect(&src);
        if let Ok((_, compat)) = &detected {
            self.compat = *compat;
        }
        let store = Store::new(self.cache.packages());
        let mut registry = env::registry(
            self.cache,
            &store,
            http,
            self.compat,
            "I need the list of published packages to figure out how to install things",
        )?;
        // `toOutlineReport` comes with 4.4a.
        let (old, _) = detected.map_err(|e| {
            one_line(
                "BAD ELM.JSON",
                format!("I ran into a problem with your elm.json file: {e:?}"),
            )
        })?;
        self.check_source_dirs(root.dir(), &old)?;
        let plan = install::plan(&mut registry, self.compat, package, &old)
            .map_err(|problem| self.install_problem(&registry, package, &old, problem))?;
        let (question, new) = match plan {
            Plan::AlreadyInstalled => {
                self.console.out("It is already installed!\n");
                return Ok(true);
            }
            Plan::PromoteIndirect(new) => (promote_indirect(), new),
            Plan::PromoteTest(new) => (promote_test(), new),
            Plan::App { changes, outline } => (plan_question(&changes), Outline::App(outline)),
            Plan::Pkg {
                changes, outline, ..
            } => (plan_question(&changes), Outline::Pkg(outline)),
        };
        match console::ask(self.console, &question, self.compat) {
            Err(e) => Err(one_line("PROBLEM READING ANSWER", e.to_string())),
            Ok(None) => {
                console::no_answer(self.console, self.personality);
                Ok(false)
            }
            Ok(Some(false)) => {
                self.console.out("Okay, I did not change anything!\n");
                Ok(true)
            }
            Ok(Some(true)) => {
                self.write(&path, &new.write())?;
                if let Err(problem) = verify(&mut registry, &store, http, self.compat, &new) {
                    self.write(&path, &src)?;
                    return Err(problem);
                }
                self.console.out("Success!\n");
                Ok(true)
            }
        }
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), Report> {
        self.vfs.write(path, bytes).map_err(|e| {
            one_line(
                "PROBLEM WRITING ELM.JSON",
                format!("I could not write elm.json: {e}"),
            )
        })
    }

    /// `Outline.read`'s check that an application's source directories
    /// exist. `toOutlineReport` comes with 4.4a.
    fn check_source_dirs(&self, root: &Path, outline: &Outline) -> Result<(), Report> {
        let Outline::App(app) = outline else {
            return Ok(());
        };
        let missing: Vec<String> = app
            .source_dirs
            .iter()
            .filter_map(|dir| dir.path())
            .filter(|dir| !root.join(dir).is_dir())
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        Err(Report::new(
            "MISSING SOURCE DIRECTORY",
            Some(ELM_JSON.to_owned()),
            &format!(
                "I need your source directories to exist, but I cannot find: {}",
                missing.join(", ")
            ),
            [],
        ))
    }

    /// `InstallNoOutline`.
    fn no_outline(&self) -> Report {
        Report::new(
            "NEW PROJECT?",
            None,
            "Are you trying to start a new project? Try this command instead:",
            [
                text(self.personality.command("init"))
                    .color(Color::Green)
                    .indent(4),
                reflow("It will help you get started!"),
            ],
        )
    }

    /// `InstallNoArgs`, naming nomi's package store where upstream names
    /// `ELM_HOME`.
    fn no_args(&self) -> Report {
        let install = self.personality.command("install");
        let make = self.personality.command("make");
        let words = |s: &'static str| s.split_whitespace().map(text);
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
                    .chain([
                        text(self.cache.packages().display().to_string()).color(Color::DullYellow)
                    ])
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
        &self,
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
            InstallProblem::NoSolution => no_solution(
                package,
                matches!(outline, Outline::App(_)),
                registry.connection,
            ),
            InstallProblem::Solver(problem) => report::solver_problem(&problem),
        }
    }
}

/// `Details.verifyInstall`, short of building: the new outline's
/// dependencies check out and are downloaded.
fn verify(
    registry: &mut Registry<'_, impl Http + Sync>,
    store: &Store,
    http: &(impl Http + Sync),
    compat: Compat,
    outline: &Outline,
) -> Result<(), Report> {
    let solution = details::verify(registry, compat, outline).map_err(details_problem)?;
    let releases = solution
        .into_iter()
        .map(|(name, details)| (name, details.version))
        .collect::<Vec<_>>();
    let names = releases.clone();
    let fetched = packages::fetch(Pool::global(), store, http, compat, releases);
    let failed: Vec<String> = names
        .iter()
        .zip(fetched)
        .filter_map(|((name, version), result)| {
            result
                .err()
                .map(|problem| format!("{name} {version}: {}", package_problem(&problem)))
        })
        .collect();
    if failed.is_empty() {
        Ok(())
    } else {
        Err(one_line(
            "PROBLEM DOWNLOADING PACKAGE",
            format!("I could not download {}", failed.join("; ")),
        ))
    }
}

/// Why a project's dependencies don't check out. `toDetailsReport` comes
/// with 4.4a.
pub(crate) fn details_problem(problem: DetailsProblem<SolverProblem>) -> Report {
    let message = match problem {
        DetailsProblem::Solver(problem) => return report::solver_problem(&problem),
        DetailsProblem::AppElm(v) => format!("This application is for elm {v}."),
        DetailsProblem::PkgElm(c) => format!("This package needs elm {c}."),
        DetailsProblem::HandEdited => {
            "The dependencies in elm.json contradict each other.".to_owned()
        }
        DetailsProblem::NoSolution => {
            "I cannot find a set of packages that satisfies elm.json.".to_owned()
        }
    };
    one_line("PROBLEM WITH DEPENDENCIES", message)
}

fn package_problem(problem: &PackageProblem) -> String {
    match problem {
        PackageProblem::BadEndpointRequest(url, e) | PackageProblem::BadArchiveRequest(url, e) => {
            format!("{url}: {e}")
        }
        PackageProblem::BadEndpointContent(url) => format!("{url} is not what I expected"),
        PackageProblem::BadArchiveContent(url) => format!("{url} is not a package archive"),
        PackageProblem::BadArchiveHash(url, expected, actual) => {
            format!("{url} has hash {actual}, not {expected}")
        }
        PackageProblem::Io(e) => e.to_string(),
    }
}

/// The package argument. Upstream's argument parser reports a bad one
/// with examples, which come with the rest of its usage errors.
pub(crate) fn parse_name(arg: &str) -> Result<Name, Report> {
    let compat = Compat::LATEST;
    Name::parse(arg.as_bytes(), Cursor::start(compat), compat).map_err(|_| {
        one_line(
            "BAD PACKAGE NAME",
            format!("{arg} is not a package name, which looks like elm/http."),
        )
    })
}

/// A report whose message is one sentence, until 4.4a ports upstream's.
pub(crate) fn one_line(title: &'static str, message: String) -> Report {
    Report::new(title, None, &message, [])
}

pub(crate) fn words(text: &str) -> impl Iterator<Item = Doc> + '_ {
    text.split_whitespace().map(Doc::text)
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
    let width = |s: String| s.chars().count();
    let (mut name_w, mut left_w, mut right_w) = (0, 0, 0);
    for (name, change) in changes {
        name_w = name_w.max(width(name.to_string()));
        match change {
            Change::Insert(v) | Change::Remove(v) => left_w = left_w.max(width(v.to_string())),
            Change::Change(old, new) => {
                left_w = left_w.max(width(old.to_string()));
                right_w = right_w.max(width(new.to_string()));
            }
        }
    }
    let pad = |w: usize, v: &T| {
        let v = v.to_string();
        text(format!(
            "{}{v}",
            " ".repeat(w.saturating_sub(width(v.clone())))
        ))
    };
    let name = |name: &Name| text(name.to_string()).fill(name_w + 3);
    let (mut inserts, mut changed, mut removes) = (Vec::new(), Vec::new(), Vec::new());
    for (n, change) in changes {
        match change {
            Change::Insert(v) => inserts.push(name(n).space(pad(left_w, v))),
            Change::Change(old, new) => changed.push(nomi_base::diagnostic::doc::hsep([
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
fn no_solution(package: &Name, app: bool, connection: Connection) -> Report {
    let existing = if app { "dependencies" } else { "constraints" };
    let start = format!(
        "I cannot find a version of {package} that is compatible with your existing {existing}."
    );
    let path = Some(ELM_JSON.to_owned());
    match (connection, app) {
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
        (Connection::Online, true) => Report::new(
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
        (Connection::Online, false) => Report::new(
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
    use std::io;
    use std::path::PathBuf;

    use nomi_registry::HttpError;

    use super::*;

    #[derive(Default)]
    struct Fake {
        out: String,
        err: String,
    }

    impl Console for Fake {
        fn out(&mut self, text: &str) {
            self.out.push_str(text);
        }
        fn err(&mut self, text: &str) {
            self.err.push_str(text);
        }
        fn read_line(&mut self) -> io::Result<Option<String>> {
            Ok(None)
        }
        fn read_all(&mut self) -> io::Result<Vec<u8>> {
            Ok(Vec::new())
        }
    }

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
        let mut console = Fake::default();
        let cache = Cache::new(PathBuf::from("/cache"));
        let succeeded = run(
            &Vfs::new(),
            &mut console,
            Personality::Elm,
            project.path(),
            &cache,
            &Unreachable,
            None,
        );
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
