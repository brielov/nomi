//! What the registry's problems say (`Reporting/Exit.hs`):
//! `toRegistryProblemReport`, `toSolverReport`, `toHttpErrorReport`, and
//! `toDetailsReport` and `toPackageProblemReport` for a project's
//! dependencies.
//!
//! nomi's HTTP client reports less than upstream's, so its errors say
//! less: a status comes with its standard reason and without the body
//! 0.19.3 shows, and other failures with the client's message.

use nomi_base::diagnostic::doc::{
    Color, Doc, dull_yellow, fill_sep, from_chars, green, note, red, reflow, text, vcat, words,
};
use nomi_base::{Compat, Report};
use nomi_project::report::{self, ElmCommand, elm_json_path};
use nomi_project::{Constraint, Name, Version};

use crate::details::DetailsProblem;
use crate::env::SolverProblem;
use crate::http::HttpError;
use crate::index::RegistryProblem;
use crate::store::PackageProblem;

/// Upstream's guess at what corrupted a response.
pub const INTERCEPTED: &str = "We have gotten reports that schools, businesses, airports, etc. \
                           sometimes intercept requests and add things to the body or change its \
                           contents entirely. Could that be the problem?";

/// `toDetailsReport` for what checking the dependencies of a project for
/// `compat` found, naming elm's commands as `command` does. `reactor` is
/// the command line that runs elm's reactor, which 0.19.1 also suggests
/// when there is one.
pub fn details_problem(
    problem: &DetailsProblem<SolverProblem>,
    compat: Compat,
    command: &dyn Fn(ElmCommand) -> String,
    reactor: Option<&str>,
) -> Report {
    let path = elm_json_path();
    let install = command(ElmCommand::Install);
    let tools = || match (reactor, compat) {
        (Some(reactor), Compat::V0_19_1) => [green(install.as_str())]
            .into_iter()
            .chain(words("or the dependency management tool in"))
            .chain([green(reactor).append(text("."))])
            .collect(),
        (None, _) | (Some(_), Compat::V0_19_2 | Compat::V0_19_3) => {
            vec![green(install.as_str()).append(text("."))]
        }
    };
    let reliable = |start: &str| {
        fill_sep(
            words(start)
                .chain(words("It is much more reliable to add dependencies with"))
                .chain(tools()),
        )
    };
    let forums = reflow(
        "Please ask for help on the community forums if you try those paths and are still \
         having problems!",
    );
    match problem {
        DetailsProblem::NoSolution => Report::new(
            "INCOMPATIBLE DEPENDENCIES",
            path,
            "The dependencies in your elm.json are not compatible.",
            [
                reliable("Did you change them by hand? Try to change it back!"),
                forums,
            ],
        ),
        DetailsProblem::NoOfflineSolution => Report::new(
            "TROUBLE VERIFYING DEPENDENCIES",
            path,
            "I could not connect to https://package.elm-lang.org to get the latest list of \
             packages, and I was unable to verify your dependencies with the information I have \
             cached locally.",
            [
                reflow(
                    "Are you able to connect to the internet? These dependencies may work once \
                     you get access to the registry!",
                ),
                note(
                    words("If you changed your dependencies by hand, try to change them back!")
                        .chain(words("It is much more reliable to add dependencies with"))
                        .chain(tools()),
                ),
            ],
        ),
        DetailsProblem::Solver(problem) => solver_problem(problem),
        DetailsProblem::Unbundled {
            name,
            wanted,
            bundled,
        } => unbundled(name, wanted, *bundled, compat),
        DetailsProblem::PkgElm(constraint) => report::pkg_elm(constraint, compat),
        DetailsProblem::AppElm(version) => report::app_elm(*version, compat),
        DetailsProblem::HandEdited => Report::new(
            "ERROR IN DEPENDENCIES",
            path,
            "It looks like the dependencies elm.json in were edited by hand (or by a 3rd party \
             tool) leaving them in an invalid state.",
            [
                reliable("Try to change them back to what they were before!"),
                forums,
            ],
        ),
    }
}

/// The `Unbundled` problem: the project asks for `wanted` of `name`, one
/// of nomi's own, and nomi bundles `bundled`.
fn unbundled(name: &Name, wanted: &Constraint, bundled: Version, compat: Compat) -> Report {
    let exact = wanted.lower == wanted.upper;
    let asked = if exact {
        format!("{name} {}", wanted.lower)
    } else {
        format!("{name} {wanted}")
    };
    let instead = if exact {
        bundled.to_string()
    } else {
        Constraint::until_next_major(bundled, compat).to_string()
    };
    Report::new(
        "PACKAGE NOT BUNDLED",
        elm_json_path(),
        &format!(
            "Your elm.json asks for {asked}, but {name} comes with nomi, never from the \
             package registry, and this nomi bundles {name} {bundled}."
        ),
        [
            reflow("Ask for that one instead:"),
            green(format!("\"{name}\": \"{instead}\"")).indent(4),
            reflow(if bundled < wanted.lower {
                "This nomi is older than the release asked for, which a newer nomi may \
                 bundle."
            } else {
                "A new major version changes the package's API, so code written for \
                 the old one may need changes too."
            }),
        ],
    )
}

/// `toPackageProblemReport`: `name` `version` couldn't be had.
pub fn package_problem(name: &Name, version: Version, problem: &PackageProblem) -> Report {
    const TITLE: &str = "PROBLEM DOWNLOADING PACKAGE";
    let package = format!("{name} {version}");
    match problem {
        PackageProblem::BadEndpointRequest(url, e) => http_error(
            TITLE,
            url,
            e,
            &format!("I need to find the latest download link for {package}"),
        ),
        PackageProblem::BadEndpointContent(url) => Report::new(
            TITLE,
            None,
            &format!(
                "I need to find the latest download link for {package}, but I ran into \
                 corrupted information from:"
            ),
            [
                url_doc(url),
                reflow(&format!(
                    "Is something weird with your internet connection. {INTERCEPTED}"
                )),
            ],
        ),
        PackageProblem::BadArchiveRequest(url, e) => http_error(
            TITLE,
            url,
            e,
            &format!("I was trying to download the source code for {package}"),
        ),
        PackageProblem::BadArchiveContent(url) => Report::new(
            TITLE,
            None,
            &format!("I downloaded the source code for {package} from:"),
            [
                url_doc(url),
                reflow(&format!(
                    "But I was unable to unzip the data. Maybe there is something weird with \
                     your internet connection. {INTERCEPTED}"
                )),
            ],
        ),
        PackageProblem::BadArchiveHash {
            url,
            expected,
            actual,
        } => Report::new(
            "CORRUPT PACKAGE DATA",
            None,
            &format!("I downloaded the source code for {package} from:"),
            [
                url_doc(url),
                reflow("But it looks like the hash of the archive has changed since publication:"),
                vcat([
                    text(format!("  Expected: {expected}")),
                    text(format!("    Actual: {actual}")),
                ]),
                reflow(
                    "This usually means that the package author moved the version tag, so \
                     report it to them and see if that is the issue. Folks on Elm slack can \
                     probably help as well.",
                ),
            ],
        ),
        PackageProblem::NotBundled => Report::new(
            "PACKAGE NOT BUNDLED",
            None,
            &format!(
                "{name} comes with nomi, never from the package registry, and this nomi \
                 does not bundle {package}."
            ),
            [],
        ),
        // nomi's own: upstream dies on a file it can't read or write.
        PackageProblem::Io(e) => Report::new(
            TITLE,
            None,
            &format!("I could not store the source code for {package}:"),
            [from_chars(&e.to_string()).indent(4)],
        ),
    }
}

/// `toRegistryProblemReport`: the index couldn't be had, which `context`
/// needed.
pub fn registry_problem(title: &'static str, problem: &RegistryProblem, context: &str) -> Report {
    match problem {
        RegistryProblem::Http(url, e) => http_error(title, url, e, context),
        RegistryProblem::Data(url, body) => unexpected_body(title, url, body, context),
    }
}

/// The `Data` case of `toRegistryProblemReport`, which publishing's
/// `PublishCannotGetTagData` repeats: `body`, fetched from `url` as
/// `context` needed, was not what was expected.
pub fn unexpected_body(title: &'static str, url: &str, body: &[u8], context: &str) -> Report {
    let whole = body.len() <= 76;
    let shown = String::from_utf8_lossy(body);
    let shown = if whole {
        shown.into_owned()
    } else {
        shown.chars().take(73).chain("...".chars()).collect()
    };
    Report::new(
        title,
        None,
        &format!("{context}, so I fetched:"),
        [
            url_doc(url),
            reflow(&format!(
                "I got the data back, but it was not what I was expecting. The response body \
                 contains {} bytes. Here is the {}",
                body.len(),
                if whole { "whole thing:" } else { "beginning:" }
            )),
            from_chars(&shown).color(Color::DullYellow).indent(4),
            reflow(&format!(
                "Does this error keep showing up? Maybe there is something weird with your \
                 internet connection. {INTERCEPTED}"
            )),
        ],
    )
}

/// `toSolverReport`.
pub fn solver_problem(problem: &SolverProblem) -> Report {
    const TITLE: &str = "PROBLEM SOLVING PACKAGE CONSTRAINTS";
    let need = |name: &Name, version: &Version| {
        format!(
            "I need the elm.json of {name} {version} to help me search for a set of compatible \
             packages"
        )
    };
    match problem {
        SolverProblem::BadCacheData(name, version) => Report::new(
            TITLE,
            None,
            &format!(
                "{}. I had it cached locally, but it looks like the file was corrupted!",
                need(name, version)
            ),
            [reflow(
                "I deleted the cached version, so the next run should download a fresh copy. \
                 Hopefully that will get you unstuck, but it will not resolve the root problem \
                 if a 3rd party tool is modifing cached files for some reason.",
            )],
        ),
        SolverProblem::BadHttpData { name, version, url } => Report::new(
            TITLE,
            None,
            &format!(
                "{}, but I ran into corrupted information from:",
                need(name, version)
            ),
            [
                url_doc(url),
                reflow(&format!(
                    "Is something weird with your internet connection. {INTERCEPTED}"
                )),
            ],
        ),
        SolverProblem::BadHttp {
            name,
            version,
            url,
            error,
        } => http_error(TITLE, url, error, &need(name, version)),
        SolverProblem::Io(e) => Report::new(
            TITLE,
            None,
            "I need the elm.json of a package to help me search for a set of compatible \
             packages, but I could not read it:",
            [from_chars(&e.to_string()).indent(4)],
        ),
    }
}

/// `toHttpErrorReport`: fetching `url` failed, which `context` needed.
pub fn http_error(title: &'static str, url: &str, e: &HttpError, context: &str) -> Report {
    let intro = format!("{context}, so I tried to fetch:");
    let details = match e {
        HttpError::Status(code) => {
            let reason = ureq::http::StatusCode::from_u16(*code)
                .ok()
                .and_then(|status| status.canonical_reason())
                .unwrap_or("");
            vec![
                fill_sep(
                    words("But it came back as")
                        .chain([red(code.to_string())])
                        .chain(words(reason)),
                ),
                reflow(
                    "This may mean some online endpoint changed in an unexpected way, so if \
                     does not seem like something on your side is causing this (e.g. firewall) \
                     please report this to https://github.com/elm/compiler/issues with your \
                     operating system, Elm version, the command you ran, the terminal output, \
                     and any additional information that can help others reproduce the error!",
                ),
            ]
        }
        HttpError::Transport(message) => vec![
            reflow("But my HTTP library is giving me the following error message:"),
            from_chars(message).indent(4),
            reflow(
                "Are you somewhere with a slow internet connection? Or no internet? Does the \
                 link I am trying to fetch work in your browser? Maybe the site is down? Does \
                 your internet connection have a firewall that blocks certain domains? It is \
                 usually something like that!",
            ),
        ],
    };
    Report::new(
        title,
        None,
        &intro,
        std::iter::once(url_doc(url)).chain(details),
    )
}

/// A URL on its own line, as reports show the one they fetched.
fn url_doc(url: &str) -> Doc {
    dull_yellow(url).indent(4)
}

#[cfg(test)]
mod tests;
