//! What the registry's problems say (`Reporting/Exit.hs`):
//! `toRegistryProblemReport`, `toSolverReport` and `toHttpErrorReport`.
//!
//! nomi's HTTP client reports less than upstream's, so its errors say
//! less: a status comes with its standard reason and without the body
//! 0.19.3 shows, and other failures with the client's message.

use nomi_base::Report;
use nomi_base::diagnostic::doc::{Color, Doc, fill_sep, reflow, text};

use crate::http::HttpError;
use crate::index::RegistryProblem;
use crate::registry::SolverProblem;

/// `toRegistryProblemReport`: the index couldn't be had, which `context`
/// needed.
pub fn registry_problem(title: &'static str, problem: &RegistryProblem, context: &str) -> Report {
    match problem {
        RegistryProblem::Http(url, e) => http_error(title, url, e, context),
        RegistryProblem::Data(url, body) => {
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
                        "I got the data back, but it was not what I was expecting. The response \
                         body contains {} bytes. Here is the {}",
                        body.len(),
                        if whole { "whole thing:" } else { "beginning:" }
                    )),
                    verbatim(&shown).color(Color::DullYellow).indent(4),
                    reflow(
                        "Does this error keep showing up? Maybe there is something weird with \
                         your internet connection. We have gotten reports that schools, \
                         businesses, airports, etc. sometimes intercept requests and add things \
                         to the body or change its contents entirely. Could that be the problem?",
                    ),
                ],
            )
        }
    }
}

/// `toSolverReport`.
pub fn solver_problem(problem: &SolverProblem) -> Report {
    const TITLE: &str = "PROBLEM SOLVING PACKAGE CONSTRAINTS";
    match problem {
        SolverProblem::BadCacheData(name, version) => Report::new(
            TITLE,
            None,
            &format!(
                "I need the elm.json of {name} {version} to help me search for a set of \
                 compatible packages. I had it cached locally, but it looks like the file was \
                 corrupted!"
            ),
            [reflow(
                "I deleted the cached version, so the next run should download a fresh copy. \
                 Hopefully that will get you unstuck, but it will not resolve the root problem \
                 if a 3rd party tool is modifing cached files for some reason.",
            )],
        ),
        SolverProblem::BadHttpData(name, version, url) => Report::new(
            TITLE,
            None,
            &format!(
                "I need the elm.json of {name} {version} to help me search for a set of \
                 compatible packages, but I ran into corrupted information from:"
            ),
            [
                url_doc(url),
                reflow(
                    "Is something weird with your internet connection. We have gotten reports \
                     that schools, businesses, airports, etc. sometimes intercept requests and \
                     add things to the body or change its contents entirely. Could that be the \
                     problem?",
                ),
            ],
        ),
        SolverProblem::BadHttp(name, version, url, e) => http_error(
            TITLE,
            url,
            e,
            &format!(
                "I need the elm.json of {name} {version} to help me search for a set of \
                 compatible packages"
            ),
        ),
        SolverProblem::Io(e) => Report::new(
            TITLE,
            None,
            "I need the elm.json of a package to help me search for a set of compatible \
             packages, but I could not read it:",
            [verbatim(&e.to_string()).indent(4)],
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
                    ["But", "it", "came", "back", "as"]
                        .into_iter()
                        .map(text)
                        .chain([text(code.to_string()).color(Color::Red)])
                        .chain(reason.split_whitespace().map(text)),
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
            verbatim(message).indent(4),
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

fn url_doc(url: &str) -> Doc {
    text(url).color(Color::DullYellow).indent(4)
}

/// `D.fromChars`: text as it is, its lines kept.
fn verbatim(text: &str) -> Doc {
    nomi_base::diagnostic::doc::vcat(text.split('\n').map(Doc::text))
}
