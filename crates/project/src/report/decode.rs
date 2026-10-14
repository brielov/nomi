//! What JSON that doesn't parse or decode says: a port of
//! `Reporting/Error/Json.hs`, for `elm.json`, the one file upstream
//! reports this way.

use nomi_base::diagnostic::code::{Position, Region, Source};
use nomi_base::diagnostic::doc::{
    Color, Doc, dull_yellow, fill_sep, green, ordinal, reflow, simple_note, stack, text, vcat,
    words,
};
use nomi_base::{Compat, Report};

use super::{elm_json_snippet, kept};
use crate::json::decode::{self, Expectation, Problem};
use crate::json::{self, ParseError, ParseErrorKind, StringProblem};

/// `ExplicitReason`, which starts every report.
const REASON: &str = "I ran into a problem with your elm.json file.";

/// `Context`: where in the value a problem is, innermost first. Only an
/// index's report says what it is in.
enum Context<'a> {
    Root,
    Field(&'a [u8]),
    Index(usize, &'a Context<'a>),
}

/// `parseErrorToReport`: the two rows before the error and its row, with
/// the error's position marked.
///
/// 0.19.1 takes the two rows before in 16 bits, so for an error on the
/// first row it starts at row 65,535 and dies showing no lines; nomi shows
/// the row, as later releases do. 0.19.1 also dies on a string byte that
/// can't start UTF-8, which nomi reports as later releases do.
#[expect(
    clippy::too_many_lines,
    reason = "upstream's `parseErrorToReport`, one message per error"
)]
pub(super) fn parse_error(source: &Source<'_>, compat: Compat, error: &ParseError) -> Report {
    let first = compat.first_position();
    let at = kept(
        json::Region {
            start: error.at,
            end: error.at,
        },
        compat,
    );
    let surroundings = Region {
        start: Position {
            row: at.start.row.saturating_sub(2).max(first),
            col: first,
        },
        end: at.start,
    };
    let snippet = |title, problem: &str, details| {
        let before = reflow(&format!("{REASON} {problem}"));
        elm_json_snippet(
            source,
            compat,
            title,
            before,
            surroundings,
            Some(at),
            details,
        )
    };
    let partway_object = "I was partway through parsing a JSON object when I got stuck here:";
    match error.kind {
        ParseErrorKind::Start => snippet(
            "EXPECTING A VALUE",
            "I was expecting to see a JSON value next:",
            stack([
                fill_sep(
                    words("Try something like")
                        .chain([dull_yellow("\"this\""), text("or"), dull_yellow("42")])
                        .chain(words("to move on to better hints!")),
                ),
                simple_note(
                    "The JSON specification does not allow trailing commas, so you can \
                     sometimes get this error in arrays that have an extra comma at the end. \
                     In that case, remove that last comma or add another array entry after it!",
                ),
            ]),
        ),
        ParseErrorKind::ObjectField => snippet(
            "EXTRA COMMA",
            partway_object,
            stack([
                fill_sep(
                    words(
                        "I saw a comma right before I got stuck here, so I was expecting to see \
                         a field name like",
                    )
                    .chain([
                        dull_yellow("\"type\""),
                        text("or"),
                        dull_yellow("\"dependencies\""),
                        text("next."),
                    ]),
                ),
                reflow(
                    "This error is commonly caused by trailing commas in JSON objects. Those \
                     are actually disallowed by <https://json.org> so check the previous line \
                     for a trailing comma that may need to be deleted.",
                ),
                object_note(),
            ]),
        ),
        ParseErrorKind::ObjectColon => snippet(
            "EXPECTING COLON",
            partway_object,
            stack([
                reflow("I was expecting to see a colon next."),
                object_note(),
            ]),
        ),
        ParseErrorKind::ObjectEnd => snippet(
            "UNFINISHED OBJECT",
            partway_object,
            stack([
                reflow("I was expecting to see a comma or a closing curly brace next."),
                reflow(
                    "Is a comma missing on the previous line? Is an array missing a closing \
                     square bracket? It is often something tricky like that!",
                ),
                object_note(),
            ]),
        ),
        ParseErrorKind::ArrayEnd => snippet(
            "UNFINISHED ARRAY",
            "I was partway through parsing a JSON array when I got stuck here:",
            stack([
                reflow("I was expecting to see a comma or a closing square bracket next."),
                reflow("Is a comma missing on the previous line? It is often something like that!"),
            ]),
        ),
        ParseErrorKind::String(StringProblem::BadStringEnd) => snippet(
            "ENDLESS STRING",
            "I got to the end of the line without seeing the closing double quote:",
            fill_sep(
                words("Strings look like")
                    .chain([green("\"this\"")])
                    .chain(words(
                        "with double quotes on each end. Is the closing double quote missing \
                         in your code?",
                    )),
            ),
        ),
        ParseErrorKind::String(StringProblem::BadStringControlChar) => snippet(
            "UNEXPECTED CONTROL CHARACTER",
            "I ran into a control character unexpectedly:",
            reflow(
                "These are characters that represent tabs, backspaces, newlines, and a bunch \
                 of other invisible characters. They all come before 20 in the ASCII range, \
                 and they are disallowed by the JSON specificaiton. Maybe a copy/paste added \
                 one of these invisible characters to your JSON?",
            ),
        ),
        ParseErrorKind::String(StringProblem::BadStringEscapeChar) => snippet(
            "UNKNOWN ESCAPE",
            "Backslashes always start escaped characters, but I do not recognize this one:",
            stack([
                reflow("Valid escape characters include:"),
                vcat(
                    [
                        "\\\"", "\\\\", "\\/", "\\b", "\\f", "\\n", "\\r", "\\t", "\\u003D",
                    ]
                    .map(text),
                )
                .indent(4)
                .color(Color::DullYellow),
                reflow(
                    "Do you want one of those instead? Maybe you need \\\\ to escape a \
                     backslash?",
                ),
            ]),
        ),
        ParseErrorKind::String(StringProblem::BadStringEscapeHex) => snippet(
            "BAD HEX ESCAPE",
            "This is not a valid hex escape:",
            fill_sep(
                words("Valid hex escapes in JSON are between")
                    .chain([green("\\u0000"), text("and"), green("\\uFFFF")])
                    .chain(words("and always have exactly four digits.")),
            ),
        ),
        ParseErrorKind::String(StringProblem::BadStringNotUtf8) | ParseErrorKind::Crash => snippet(
            "UNEXPECTED ENCODING",
            "JSON files use UTF-8 character encoding, but I ran into something outside of \
                 that format:",
            reflow("Is there a way to convert this to a valid UTF-8 character?"),
        ),
        ParseErrorKind::NoLeadingZeros => snippet(
            "BAD NUMBER",
            "Numbers cannot start with zeros like this:",
            reflow("Try deleting the leading zeros?"),
        ),
        ParseErrorKind::NoFloats => snippet(
            "UNEXPECTED NUMBER",
            "I got stuck while trying to parse this number:",
            reflow(
                "I do not accept floating point numbers like 3.1415 right now. That kind of \
                 JSON value is not needed for any of the uses that Elm has for now.",
            ),
        ),
        ParseErrorKind::BadEnd => snippet(
            "JSON PROBLEM",
            "I was partway through parsing some JSON when I got stuck here:",
            reflow(
                "I am not really sure what is wrong. This sometimes means there is extra stuff \
                 after a valid JSON value?",
            ),
        ),
    }
}

/// `objectNote`.
fn object_note() -> Doc {
    stack([
        simple_note("Here is an example of a valid JSON object for reference:"),
        vcat([
            text("{").indent(4),
            dull_yellow("\"name\"")
                .append(text(": "))
                .append(dull_yellow("\"Tom\""))
                .append(text(","))
                .indent(6),
            dull_yellow("\"age\"")
                .append(text(": "))
                .append(dull_yellow("42"))
                .indent(6),
            text("}").indent(4),
        ]),
        reflow(
            "Notice that (1) the field names are in double quotes and (2) there is no trailing \
             comma after the last entry. Both are strict requirements in JSON!",
        ),
    ])
}

/// `problemToReport`: the deepest of the problems, with `failure`
/// reporting the caller's own.
pub(super) fn problem<X>(
    source: &Source<'_>,
    compat: Compat,
    problem: &Problem<X>,
    failure: &dyn Fn(json::Region, &X) -> Report,
) -> Report {
    report(source, compat, &Context::Root, problem, failure)
}

fn report<X>(
    source: &Source<'_>,
    compat: Compat,
    context: &Context<'_>,
    problem: &Problem<X>,
    failure: &dyn Fn(json::Region, &X) -> Report,
) -> Report {
    match problem {
        Problem::Field(field, p) => report(source, compat, &Context::Field(field), p, failure),
        Problem::Index(index, p) => {
            report(source, compat, &Context::Index(*index, context), p, failure)
        }
        Problem::OneOf(p, ps) => report(source, compat, context, decode::deepest(p, ps), failure),
        Problem::Failure(region, x) => failure(*region, x),
        Problem::Expecting(region, expectation) => {
            expectation_report(source, compat, context, *region, expectation)
        }
    }
}

/// `expectationToReport`. Upstream defines the region it shows of a value
/// on one row as itself, and so hangs after the report's first paragraph;
/// nomi shows the value, which is what the definition says otherwise.
/// A value over several rows shows its first position, as upstream does.
fn expectation_report(
    source: &Source<'_>,
    compat: Compat,
    context: &Context<'_>,
    region: json::Region,
    expectation: &Expectation,
) -> Report {
    let region = kept(region, compat);
    let region = if region.start.row == region.end.row {
        region
    } else {
        Region {
            start: region.start,
            end: region.start,
        }
    };
    let field_name = |field: &[u8]| String::from_utf8_lossy(field).into_owned();
    let introduction = match context {
        Context::Root => "I ran into some trouble here:".to_owned(),
        Context::Field(field) => format!(
            "I ran into trouble with the value of the \"{}\" field:",
            field_name(field)
        ),
        Context::Index(index, Context::Field(field)) => format!(
            "When looking at the \"{}\" field, I ran into trouble with the {} entry:",
            field_name(field),
            ordinal(*index)
        ),
        Context::Index(index, Context::Root | Context::Index(..)) => format!(
            "I ran into trouble with the {} index of this array:",
            ordinal(*index)
        ),
    };
    let snippet = |title, a_thing: Vec<Doc>| {
        let before = reflow(&format!("{REASON} {introduction}"));
        let after = fill_sep(words("I was expecting to run into").chain(a_thing));
        elm_json_snippet(source, compat, title, before, region, None, after)
    };
    let a = |article: &str, what: &str| vec![text(article), green(what).append(text("."))];
    match expectation {
        Expectation::Object => snippet("EXPECTING OBJECT", a("an", "OBJECT")),
        Expectation::Array => snippet("EXPECTING ARRAY", a("an", "ARRAY")),
        Expectation::String => snippet("EXPECTING STRING", a("a", "STRING")),
        Expectation::ObjectWith(field) => snippet(
            "MISSING FIELD",
            vec![
                text("an"),
                green("OBJECT"),
                text("with"),
                text("a"),
                green(format!("\"{}\"", field_name(field))),
                text("field."),
            ],
        ),
    }
}
