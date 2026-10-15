//! nomi invoked as `elm`: the Elm compiler's commands, as they come.
//! What they do is `nomi_engine`'s; a command nomi doesn't answer to
//! says so.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use nomi_base::Compat;
use nomi_base::diagnostic::doc::Doc;
use nomi_engine::check::Style;
use nomi_engine::console::{self, Console, Personality, StdConsole};
use nomi_engine::javascript::Target;
use nomi_engine::make::{self, Make, Output};
use nomi_engine::{bump, publish};

use crate::terminal::{self, Chunks, Parser};

pub(crate) fn main(args: impl Iterator<Item = OsString>) -> ExitCode {
    let args: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let succeeded = match args[..] {
        ["init"] => crate::new(Personality::Elm, None),
        ["install"] => crate::add(Personality::Elm, None),
        ["install", package] => crate::add(Personality::Elm, Some(package)),
        ["make", ref rest @ ..] => make(rest),
        ["diff", ref rest @ ..] => crate::diff(
            rest,
            "`elm diff` takes no arguments, a version, two versions, or a package and two versions",
        ),
        ["repl", ..] => {
            eprintln!(
                "error: nomi has no REPL, so `elm repl` isn't here; elm's own still works beside it"
            );
            false
        }
        ["reactor", ..] => {
            eprintln!(
                "error: nomi has no reactor; `nomi run --watch` rebuilds and serves your entries as you edit"
            );
            false
        }
        ["publish"] => crate::in_project(publish::run),
        ["bump"] => crate::in_project(bump::run),
        _ => {
            eprintln!("error: nomi doesn't answer to `elm {}` yet", args.join(" "));
            false
        }
    };
    crate::exit(succeeded)
}

/// `elm make`, or `exitWithError` where its arguments don't parse.
fn make(args: &[&str]) -> bool {
    let make = match parse_make(args) {
        Ok(make) => make,
        Err(error) => {
            let mut console = StdConsole;
            for doc in error.to_docs() {
                console::report_doc(&mut console, &doc.append(Doc::line()), Compat::LATEST);
                console.err("\n");
            }
            return false;
        }
    };
    crate::in_project(|cx, console| make::run(cx, console, &make))
}

/// `elm make`'s arguments, parsed as `Terminal.Chomp` parses them, its
/// flags in the order `Main.hs` declares them.
fn parse_make(args: &[&str]) -> Result<Make, terminal::Error> {
    let mut chunks = Chunks::new(args);
    let debug = chunks.on_off("debug")?;
    let optimize = chunks.on_off("optimize")?;
    let output = chunks.value(
        "output",
        &Parser {
            singular: "output file",
            parse: Output::parse,
            examples: &["elm.js", "index.html", "/dev/null"],
        },
    )?;
    let report = chunks.value(
        "report",
        &Parser {
            singular: "report type",
            parse: |s| (s == "json").then_some(Style::Json),
            examples: &["json"],
        },
    )?;
    let docs = chunks.value(
        "docs",
        &Parser {
            singular: "json file",
            parse: |s| make::has_ext(".json", s).then(|| PathBuf::from(s)),
            examples: &["docs.json", "documentation.json"],
        },
    )?;
    let paths = chunks.zero_or_more(&Parser {
        singular: "elm file",
        parse: |s| (make::take_extension(s) == ".elm").then(|| PathBuf::from(s)),
        examples: &["Main.elm", "src/Main.elm"],
    })?;
    Ok(Make {
        paths,
        personality: Personality::Elm,
        style: report.unwrap_or(Style::Terminal),
        kind: make::kind(debug, optimize),
        target: Target::default(),
        output,
        docs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomi_engine::javascript::Kind;

    use crate::terminal::Error;

    #[test]
    fn flags_are_taken_wherever_they_are() {
        let make = parse_make(&["src/A.elm", "--output", "a.js", "--debug", "--report=json"]);
        let make = make.unwrap();
        assert_eq!(make.kind, Ok(Kind::Debug));
        assert_eq!(make.output, Some(Output::Js(PathBuf::from("a.js"))));
        assert_eq!(make.style, Style::Json);
        assert_eq!(make.paths, [PathBuf::from("src/A.elm")]);
        // An on/off flag leaves what follows it.
        let make = parse_make(&["--debug", "src/A.elm"]).unwrap();
        assert_eq!(make.paths, [PathBuf::from("src/A.elm")]);
    }

    #[test]
    fn the_first_flag_declared_reports_first() {
        let error = |args: &[&str]| parse_make(args).unwrap_err();
        assert!(matches!(
            error(&["--output=a.txt", "--debug=1"]),
            Error::FlagWithValue { flag: "debug", .. }
        ));
        assert!(matches!(
            error(&["--docs", "--output=a.txt"]),
            Error::FlagWithBadValue { flag: "output", .. }
        ));
        assert!(matches!(
            error(&["--docs"]),
            Error::FlagWithNoValue("docs", _)
        ));
        assert!(matches!(error(&["--zz", "a.txt"]), Error::FlagUnknown(..)));
        assert!(matches!(error(&["a.txt"]), Error::ArgBad(..)));
    }

    #[test]
    fn an_unknown_flag_names_the_declared_ones_nearest() {
        let nearby = |args: &[&str]| match parse_make(args) {
            Err(Error::FlagUnknown(_, nearby)) => nearby,
            other => panic!("{other:?}"),
        };
        assert_eq!(nearby(&["--repor=json"]), ["--report=<report-type>"]);
        assert_eq!(
            nearby(&["--qqqqqqqq"]),
            [
                "--debug",
                "--optimize",
                "--output=<output-file>",
                "--report=<report-type>",
                "--docs=<json-file>"
            ]
        );
    }
}
