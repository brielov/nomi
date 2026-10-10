//! nomi invoked as `elm`: the Elm compiler's commands, as they come.
//! What they do is `nomi_engine`'s; commands later milestones implement
//! say they aren't there yet.

use std::ffi::OsString;
use std::process::ExitCode;

use std::path::PathBuf;

use nomi_base::Compat;
use nomi_base::diagnostic::doc::Doc;
use nomi_engine::Vfs;
use nomi_engine::check::Style;
use nomi_engine::console::{self, Console, Personality, StdConsole};
use nomi_engine::make::{self, Make, Output};

use crate::terminal::{self, Chunks, Flag, Parser};

pub fn main(args: impl Iterator<Item = OsString>) -> ExitCode {
    let args: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let succeeded = match args[..] {
        ["init"] => crate::new(Personality::Elm, None),
        ["install"] => crate::add(Personality::Elm, None),
        ["install", package] => crate::add(Personality::Elm, Some(package)),
        ["make", ref rest @ ..] => make(rest),
        _ => {
            eprintln!("error: nomi doesn't answer to `elm {}` yet", args.join(" "));
            false
        }
    };
    crate::exit(succeeded)
}

/// `make`'s flags, in the order `Main.hs` declares them.
const MAKE_FLAGS: [Flag; 5] = [
    Flag::OnOff("debug"),
    Flag::OnOff("optimize"),
    Flag::Value("output", "output file"),
    Flag::Value("report", "report type"),
    Flag::Value("docs", "json file"),
];

/// `elm make`, its arguments parsed as `Terminal.Chomp` parses them.
fn make(args: &[&str]) -> bool {
    let parsed = (|| {
        let mut chunks = Chunks::new(args, &MAKE_FLAGS);
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
        Ok::<_, terminal::Error>(Make {
            paths,
            personality: Personality::Elm,
            style: report.unwrap_or(Style::Terminal),
            debug,
            optimize,
            output,
            docs,
        })
    })();
    let make = match parsed {
        Ok(make) => make,
        Err(error) => {
            let docs = error.to_docs();
            let mut console = StdConsole;
            for doc in docs {
                console::report_doc(&mut console, &doc.append(Doc::line()), Compat::LATEST);
                console.err("\n");
            }
            return false;
        }
    };
    crate::in_project(|console, dir, cache, http| {
        make::run(&Vfs::new(), console, dir, cache, http, &make)
    })
}
