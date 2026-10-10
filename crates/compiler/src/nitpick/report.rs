//! The reports of pattern errors (`Reporting/Error/Pattern.hs`).

use nomi_base::diagnostic::Problem;
use nomi_base::diagnostic::code::{self, Source};
use nomi_base::diagnostic::doc::{
    self, Color, Doc, ordinal, reflow, simple_hint, snippet, stack, text, vcat,
};
use nomi_base::{Compat, TextRange};

use super::patterns::{Context, Error, Literal, Pattern};

/// What a report needs around its error.
pub struct ReportContext<'a> {
    pub source: &'a Source<'a>,
    pub compat: Compat,
    pub trim: &'a dyn Fn(TextRange) -> TextRange,
}

impl std::fmt::Debug for ReportContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReportContext")
            .field("compat", &self.compat)
            .finish_non_exhaustive()
    }
}

/// `toReport`.
pub fn to_problem(error: &Error, cx: &ReportContext<'_>) -> Problem {
    // 0.19.2 corrected upstream's spelling.
    let (possibilities, possibility) = match cx.compat {
        Compat::V0_19_1 => ("possiblities", "possiblity"),
        Compat::V0_19_2 | Compat::V0_19_3 => ("possibilities", "possibility"),
    };
    let snip = |region: TextRange, highlight: Option<TextRange>, before: Doc, after: Doc| {
        snippet(before, (cx.trim)(region), highlight.map(cx.trim), after)
    };
    let (title, range, message) = match error {
        Error::Redundant(case_region, pattern_region, index) => (
            "REDUNDANT PATTERN",
            *pattern_region,
            snip(
                *case_region,
                Some(*pattern_region),
                reflow(&format!("The {} pattern is redundant:", ordinal(*index))),
                reflow(
                    "Any value with this shape will be handled by a previous pattern, so it should be removed.",
                ),
            ),
        ),
        Error::Incomplete(region, Context::BadArg, unhandled) => (
            "UNSAFE PATTERN",
            *region,
            snip(
                *region,
                None,
                text(format!("This pattern does not cover all {possibilities}:")),
                stack([
                    text("Other possibilities include:"),
                    unhandled_block(unhandled),
                    reflow(
                        "I would have to crash if I saw one of those! So rather than pattern matching in function arguments, put a `case` in the function body to account for all possibilities.",
                    ),
                ]),
            ),
        ),
        Error::Incomplete(region, Context::BadDestruct, unhandled) => (
            "UNSAFE PATTERN",
            *region,
            snip(
                *region,
                None,
                text("This pattern does not cover all possible values:"),
                stack([
                    text("Other possibilities include:"),
                    unhandled_block(unhandled),
                    reflow(&format!(
                        "I would have to crash if I saw one of those! You can use `let` to deconstruct values only if there is ONE {possibility}. Switch to a `case` expression to account for all possibilities."
                    )),
                    simple_hint(
                        "Are you calling a function that definitely returns values with a very specific shape? Try making the return type of that function more specific!",
                    ),
                ]),
            ),
        ),
        Error::Incomplete(region, Context::BadCase, unhandled) => (
            "MISSING PATTERNS",
            *region,
            snip(
                *region,
                None,
                text("This `case` does not have branches for all possibilities:"),
                stack([
                    text("Missing possibilities include:"),
                    unhandled_block(unhandled),
                    reflow("I would have to crash if I saw one of those. Add branches for them!"),
                    doc::link(
                        "Hint",
                        "If you want to write the code for each branch later, use `Debug.todo` as a placeholder. Read",
                        "missing-patterns",
                        "for more guidance on this workflow.",
                    ),
                ]),
            ),
        ),
    };
    let range = (cx.trim)(range);
    let region = cx.source.region(range, cx.compat);
    Problem::new(title, region, code::expand(&message, cx.source, cx.compat))
}

/// `unhandledPatternsToDocBlock`.
fn unhandled_block(patterns: &[Pattern]) -> Doc {
    vcat(
        patterns
            .iter()
            .map(|p| text(to_text(Place::Unambiguous, p))),
    )
    .color(Color::DullYellow)
    .indent(4)
}

/// `Context`: where a pattern prints, which decides its parentheses.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Arg,
    Head,
    Unambiguous,
}

/// The text between a literal's quotes.
fn inner(text: &str, quote: char) -> &str {
    text.strip_prefix(quote)
        .and_then(|t| t.strip_suffix(quote))
        .unwrap_or(text)
}

/// `patternToDoc`.
fn to_text(place: Place, pattern: &Pattern) -> String {
    match delist(pattern) {
        Structure::NonList(Pattern::Anything) => "_".to_owned(),
        Structure::NonList(Pattern::Literal(literal)) => match literal {
            Literal::Chr(c) => format!("'{}'", inner(c, '\'')),
            Literal::Str(s) => {
                let s = s
                    .strip_prefix("\"\"\"")
                    .and_then(|t| t.strip_suffix("\"\"\""))
                    .unwrap_or(inner(s, '"'));
                format!("\"{s}\"")
            }
            Literal::Int(n) => n.to_string(),
        },
        Structure::NonList(Pattern::Ctor(_, name, args)) => match (name.as_str(), &args[..]) {
            ("#0", []) => "()".to_owned(),
            ("#2", [a, b]) => format!(
                "( {}, {} )",
                to_text(Place::Unambiguous, a),
                to_text(Place::Unambiguous, b)
            ),
            ("#3", [a, b, c]) => format!(
                "( {}, {}, {} )",
                to_text(Place::Unambiguous, a),
                to_text(Place::Unambiguous, b),
                to_text(Place::Unambiguous, c)
            ),
            _ => {
                let ctor = std::iter::once(name.clone())
                    .chain(args.iter().map(|a| to_text(Place::Arg, a)))
                    .collect::<Vec<_>>()
                    .join(" ");
                if place == Place::Arg && !args.is_empty() {
                    format!("({ctor})")
                } else {
                    ctor
                }
            }
        },
        Structure::FiniteList(entries) if entries.is_empty() => "[]".to_owned(),
        Structure::FiniteList(entries) => {
            let docs: Vec<String> = entries
                .iter()
                .map(|e| to_text(Place::Unambiguous, e))
                .collect();
            format!("[{}]", docs.join(","))
        }
        Structure::Conses(conses, last) => {
            let doc = (conses.iter().rev())
                .fold(to_text(Place::Unambiguous, last), |tail, head| {
                    format!("{} :: {tail}", to_text(Place::Head, head))
                });
            if place == Place::Unambiguous {
                doc
            } else {
                format!("({doc})")
            }
        }
    }
}

/// `Structure`.
enum Structure<'p> {
    FiniteList(Vec<&'p Pattern>),
    Conses(Vec<&'p Pattern>, &'p Pattern),
    NonList(&'p Pattern),
}

/// `delist`. A finite list's entries come out reversed, as upstream's do.
fn delist(pattern: &Pattern) -> Structure<'_> {
    let mut rev_entries: Vec<&Pattern> = Vec::new();
    let mut at = pattern;
    loop {
        match at {
            Pattern::Ctor(_, name, args) if name == "[]" && args.is_empty() => {
                // `FiniteList revEntries`: upstream's list of them, latest
                // first.
                rev_entries.reverse();
                return Structure::FiniteList(rev_entries);
            }
            Pattern::Ctor(_, name, args) if name == "::" && args.len() == 2 => {
                rev_entries.push(&args[0]);
                at = &args[1];
            }
            _ if rev_entries.is_empty() => return Structure::NonList(at),
            _ => return Structure::Conses(rev_entries, at),
        }
    }
}
