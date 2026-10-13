//! What a bad `elm.json` says (`Reporting/Exit.hs`): `toOutlineReport`,
//! with the snippets of `toOutlineProblemReport`, and the version checks
//! of `toDetailsReport`.
//!
//! Reports name the commands they suggest through `command`, which turns
//! elm's command (`install`, `init`) into the command line that runs it,
//! so nomi's own names show under its own name.

use nomi_base::diagnostic::code::{Position, Region, Source};
use nomi_base::diagnostic::doc::{Color, Doc, fill_sep, reflow, simple_note, stack, text, vcat};
use nomi_base::{Compat, Report};

use crate::constraint::ConstraintError;
use crate::json;
use crate::outline::{DetectError, OutlineError, OutlineProblem};
use crate::{Constraint, Version};

pub mod decode;

/// The path reports about the outline name.
const ELM_JSON: &str = "elm.json";

/// `toOutlineReport` for what reading `src` found, with `Exit.Details`'s
/// version checks for the release no supported one is: as the newest
/// release reports it, since that is the one nomi tries first.
pub fn detect(error: &DetectError, src: &[u8], command: &dyn Fn(&str) -> String) -> Report {
    match error {
        DetectError::Outline(compat, e) => outline(e, src, *compat, command),
        DetectError::AppElm(version) => app_elm(*version, Compat::LATEST),
        DetectError::PkgElm(constraint) => pkg_elm(constraint, Compat::LATEST),
    }
}

/// `toOutlineReport` for what reading `src` as `compat` found.
pub fn outline(
    error: &OutlineError,
    src: &[u8],
    compat: Compat,
    command: &dyn Fn(&str) -> String,
) -> Report {
    let missing = |start: &str, after: String| {
        Report::new("MISSING DEPENDENCY", Some(path()), start, [reflow(&after)])
    };
    let by_hand = "If you modified your elm.json by hand, try to change it back! And if you are \
                   having trouble getting back to a working elm.json, it may be easier to";
    let start_fresh = format!(
        "{by_hand} delete it and use `{}` to start fresh.",
        command("init")
    );
    let core = "I need to see an \"elm/core\" dependency your elm.json file. The default imports \
                of `List` and `Maybe` do not work without it.";
    match error {
        OutlineError::Parse(e) => {
            let text = nomi_base::ghc::lenient_utf8(src);
            decode::parse_error(&Source::new(&text), compat, e)
        }
        OutlineError::Decode(problem) => {
            let text = nomi_base::ghc::lenient_utf8(src);
            let source = Source::new(&text);
            decode::problem(&source, compat, problem, &|region, x| {
                outline_problem(&source, compat, region, x, command)
            })
        }
        OutlineError::NoPkgCore => missing(
            core,
            format!("{by_hand} find a working package and start fresh with their elm.json file."),
        ),
        OutlineError::NoAppCore => missing(core, start_fresh),
        OutlineError::NoAppJson => missing(
            "I need to see an \"elm/json\" dependency your elm.json file. It helps me handle \
             flags and ports.",
            start_fresh,
        ),
    }
}

/// `OutlineHasMissingSrcDirs`: the source directories that don't exist,
/// as `elm.json` writes them.
pub fn missing_src_dirs(dirs: &[String]) -> Report {
    let red = |dir: &String| text(dir.clone()).color(Color::Red);
    match dirs {
        [dir] => Report::new(
            "MISSING SOURCE DIRECTORY",
            Some(path()),
            "I need a valid elm.json file, but the \"source-directories\" field lists the \
             following directory:",
            [
                red(dir).indent(4),
                reflow("I cannot find it though. Is it missing? Is there a typo?"),
            ],
        ),
        _ => Report::new(
            "MISSING SOURCE DIRECTORIES",
            Some(path()),
            "I need a valid elm.json file, but the \"source-directories\" field lists the \
             following directories:",
            [
                vcat(dirs.iter().map(red)).indent(4),
                reflow("I cannot find them though. Are they missing? Are there typos?"),
            ],
        ),
    }
}

/// `OutlineHasDuplicateSrcDirs`: two source directories, as `elm.json`
/// writes them, that are the directory `canonical`.
pub fn duplicate_src_dirs(canonical: &str, dir1: &str, dir2: &str) -> Report {
    const TITLE: &str = "REDUNDANT SOURCE DIRECTORIES";
    let dirs = vcat([dir1, dir2].map(|dir| text(dir).color(Color::Red))).indent(4);
    if dir1 == dir2 {
        return Report::new(
            TITLE,
            Some(path()),
            "I need a valid elm.json file, but the \"source-directories\" field lists the same \
             directory twice:",
            [dirs, reflow("Remove one of the entries!")],
        );
    }
    Report::new(
        TITLE,
        Some(path()),
        "I need a valid elm.json file, but the \"source-directories\" field has some redundant \
         directories:",
        [
            dirs,
            reflow("These are two different ways of refering to the same directory:"),
            text(canonical).color(Color::DullYellow).indent(4),
            reflow("Remove one of the redundant entries from your \"source-directories\" field."),
        ],
    )
}

/// `DetailsBadElmInAppOutline`: the application is for `version`, not
/// for `compat`.
pub fn app_elm(version: Version, compat: Compat) -> Report {
    Report::new(
        "ELM VERSION MISMATCH",
        Some(path()),
        "Your elm.json says this application needs a different version of Elm.",
        [fill_sep([
            text("It"),
            text("requires"),
            text(version.to_string())
                .color(Color::Green)
                .append(text(",")),
            text("but"),
            text("you"),
            text("are"),
            text("using"),
            text(Version::compiler(compat).to_string()).color(Color::Red),
            text("right"),
            text("now."),
        ])],
    )
}

/// `DetailsBadElmInPkg`: the package excludes `compat`.
pub fn pkg_elm(constraint: &Constraint, compat: Compat) -> Report {
    Report::new(
        "ELM VERSION MISMATCH",
        Some(path()),
        "Your elm.json says this package needs a version of Elm in this range:",
        [
            text(constraint.to_string())
                .color(Color::DullYellow)
                .indent(4),
            fill_sep([
                text("But"),
                text("you"),
                text("are"),
                text("using"),
                text("Elm"),
                text(Version::compiler(compat).to_string()).color(Color::Red),
                text("right"),
                text("now."),
            ]),
        ],
    )
}

fn path() -> String {
    ELM_JSON.to_owned()
}

/// A region of `elm.json`, as `compat` keeps it: the JSON parser's
/// positions are the editor's, from 1.
pub(crate) fn kept(region: json::Region, compat: Compat) -> Region {
    let keep = |p: json::Position| Position {
        row: p.row.wrapping_sub(1).wrapping_add(compat.first_position()),
        col: p.col.wrapping_sub(1).wrapping_add(compat.first_position()),
    };
    Region {
        start: keep(region.start),
        end: keep(region.end),
    }
}

fn words(text: &str) -> impl Iterator<Item = Doc> + '_ {
    text.split_whitespace().map(Doc::text)
}

fn green(chars: impl Into<String>) -> Doc {
    text(chars).color(Color::Green)
}

/// `toOutlineProblemReport`: what a field's own decoder rejected, in
/// `region`.
fn outline_problem(
    source: &Source<'_>,
    compat: Compat,
    region: json::Region,
    problem: &OutlineProblem,
    command: &dyn Fn(&str) -> String,
) -> Report {
    let at = |p: json::Position| Some(kept(json::Region { start: p, end: p }, compat));
    let snippet =
        |title: &'static str, highlight: Option<Region>, before: &str, after: Doc| Report {
            title: title.into(),
            path: Some(path()),
            message: source.snippet(
                compat,
                reflow(before),
                kept(region, compat),
                highlight,
                after,
            ),
        };
    let stuck = "I got stuck while reading your elm.json file.";
    let too_long = |what: &str| {
        stack([
            fill_sep(
                words("I need it to be")
                    .chain([green("under"), green(what), green("bytes")])
                    .chain(words("so it renders nicely on the package website!")),
            ),
            simple_note(
                "I count the length in bytes, so using non-ASCII characters costs extra. \
                 Please report your case at https://github.com/elm/compiler/issues if this \
                 seems overly restrictive for your needs.",
            ),
        ])
    };
    match problem {
        OutlineProblem::BadType => snippet(
            "UNEXPECTED TYPE",
            None,
            &format!("{stuck} I cannot handle a \"type\" like this:"),
            fill_sep(
                words("Try changing the \"type\" to")
                    .chain([green("\"application\""), text("or"), green("\"package\"")])
                    .chain([text("instead.")]),
            ),
        ),
        OutlineProblem::BadPkgName(p) => snippet(
            "INVALID PACKAGE NAME",
            at(*p),
            &format!("{stuck} I ran into trouble with the package name:"),
            stack([
                fill_sep(
                    words("Package names are always written as")
                        .chain([green("\"author/project\"")])
                        .chain(words("so I am expecting to see something like:")),
                ),
                vcat(
                    [
                        "\"mdgriffith/elm-ui\"",
                        "\"w0rm/elm-physics\"",
                        "\"Microsoft/elm-json-tree-view\"",
                        "\"FordLabs/elm-star-rating\"",
                        "\"1602/json-schema\"",
                    ]
                    .map(text),
                )
                .indent(4)
                .color(Color::DullYellow),
                reflow(
                    "The author name should match your GitHub name exactly, and the project \
                     name needs to follow these rules:",
                ),
                vcat(
                    [
                        "+--------------------------------------+-----------+-----------+",
                        "| RULE                                 | BAD       | GOOD      |",
                        "+--------------------------------------+-----------+-----------+",
                        "| only lower case, digits, and hyphens | elm-HTTP  | elm-http  |",
                        "| no leading digits                    | 3D        | elm-3d    |",
                        "| no non-ASCII characters              | elm-bjørn | elm-bear  |",
                        "| no underscores                       | elm_ui    | elm-ui    |",
                        "| no double hyphens                    | elm--hash | elm-hash  |",
                        "| no starting or ending hyphen         | -elm-tar- | elm-tar   |",
                        "+--------------------------------------+-----------+-----------+",
                    ]
                    .map(text),
                )
                .indent(4),
                simple_note(
                    "These rules only apply to the project name, so you should never need to \
                     change your GitHub name!",
                ),
            ]),
        ),
        OutlineProblem::BadVersion(p) => snippet(
            "PROBLEM WITH VERSION",
            at(*p),
            &format!("{stuck} I was expecting a version number here:"),
            fill_sep(
                words("I need something like")
                    .chain([green("\"1.0.0\""), text("or"), green("\"2.0.4\"")])
                    .chain(words("that explicitly states all three numbers!")),
            ),
        ),
        OutlineProblem::BadConstraint(ConstraintError::BadFormat(p)) => snippet(
            "PROBLEM WITH CONSTRAINT",
            at(*p),
            &format!("{stuck} I do not understand this version constraint:"),
            stack([
                fill_sep(
                    words("I need something like")
                        .chain([green("\"1.0.0 <= v < 2.0.0\"")])
                        .chain(words("that explicitly lists the lower and upper bounds.")),
                ),
                simple_note(
                    "The spaces in there are required! Taking them out will confuse me. Adding \
                     extra spaces confuses me too. I recommend starting with a valid example \
                     and just changing the version numbers.",
                ),
            ]),
        ),
        OutlineProblem::BadConstraint(ConstraintError::InvalidRange(before, after)) => {
            let range = |upper: Version| green(format!("\"{before} <= v < {upper}\""));
            let after = if before == after {
                fill_sep(
                    words(
                        "Elm checks that all package APIs follow semantic versioning, so it is \
                         best to use wide constraints. I recommend",
                    )
                    .chain([range(after.bump_major(compat))])
                    .chain(words(
                        "since it is guaranteed that breaking API changes cannot happen in any \
                         of the versions in that range.",
                    )),
                )
            } else {
                fill_sep(
                    words("Maybe you want something like")
                        .chain([range(before.bump_major(compat))])
                        .chain(words(
                            "instead? Elm checks that all package APIs follow semantic \
                             versioning, so it is guaranteed that breaking API changes cannot \
                             happen in any of the versions in that range.",
                        )),
                )
            };
            snippet(
                "PROBLEM WITH CONSTRAINT",
                None,
                &format!("{stuck} I ran into an invalid version constraint:"),
                after,
            )
        }
        OutlineProblem::BadModuleName(p) => snippet(
            "PROBLEM WITH MODULE NAME",
            at(*p),
            &format!("{stuck} I was expecting a module name here:"),
            fill_sep(
                words("I need something like")
                    .chain([
                        green("\"Html.Events\""),
                        text("or"),
                        green("\"Browser.Navigation\""),
                    ])
                    .chain(words(
                        "where each segment starts with a capital letter and the segments are \
                         separated by dots.",
                    )),
            ),
        ),
        OutlineProblem::BadModuleHeaderTooLong => snippet(
            "HEADER TOO LONG",
            None,
            &format!("{stuck} This section header is too long:"),
            too_long("20"),
        ),
        OutlineProblem::BadDependencyName(p) => snippet(
            "PROBLEM WITH DEPENDENCY NAME",
            at(*p),
            &format!("{stuck} There is something wrong with this dependency name:"),
            stack([
                fill_sep(
                    words(
                        "Package names always include the name of the author, so I am \
                         expecting to see dependencies like",
                    )
                    .chain([
                        text("\"mdgriffith/elm-ui\"").color(Color::DullYellow),
                        text("and"),
                        text("\"Microsoft/elm-json-tree-view\"")
                            .color(Color::DullYellow)
                            .append(text(".")),
                    ]),
                ),
                fill_sep(
                    words(
                        "I generally recommend finding the package you want on the package \
                         website, and installing it with the",
                    )
                    .chain([green(command("install")), text("command!")]),
                ),
            ]),
        ),
        OutlineProblem::BadLicense(_, suggestions) => snippet(
            "UNKNOWN LICENSE",
            None,
            &format!("{stuck} I do not know about this type of license:"),
            stack([
                fill_sep(
                    words("Elm packages generally use")
                        .chain([
                            green("\"BSD-3-Clause\""),
                            text("or"),
                            green("\"MIT\"").append(text(",")),
                        ])
                        .chain(words(
                            "but I accept any OSI approved SPDX license. Here some that seem \
                             close to what you wrote:",
                        )),
                ),
                vcat(suggestions.iter().map(|s| text(*s)))
                    .color(Color::DullYellow)
                    .indent(4),
                reflow("Check out https://spdx.org/licenses/ for the full list of options."),
            ]),
        ),
        OutlineProblem::BadSummaryTooLong => snippet(
            "SUMMARY TOO LONG",
            None,
            &format!("{stuck} Your \"summary\" is too long:"),
            too_long("80"),
        ),
        OutlineProblem::NoSrcDirs => snippet(
            "NO SOURCE DIRECTORIES",
            None,
            &format!("{stuck} You do not have any \"source-directories\" listed here:"),
            fill_sep(
                words("I need something like")
                    .chain([green("[\"src\"]")])
                    .chain(words("so I know where to look for your modules!")),
            ),
        ),
    }
}

#[cfg(test)]
mod tests;
