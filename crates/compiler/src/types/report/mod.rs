//! The reports of type errors (`Reporting/Error/Type.hs`): annotations
//! and mismatches here, with the hints, operators, patterns, and records
//! and infinite types beside.

mod hints;
mod operators;
mod patterns;
mod records;

use nomi_base::diagnostic::Problem;
use nomi_base::diagnostic::code::{self, Source};
use nomi_base::diagnostic::doc::{
    Color, Doc, args, comma_sep, fill_sep, ordinal, reflow, simple_hint, simple_note, snippet,
    stack, text, vcat,
};
use nomi_base::{Compat, TextRange, suggest};

use super::error::{self as t, Direction, Problem as TProblem, Super, to_comparison};
use super::expected::{
    Category, Context, Expected, MaybeName, PCategory, PContext, PExpected, SubContext,
};
use super::localizer::Localizer;
use super::solve::TypeError;
use hints::problems_to_hint;
use operators::{RightDocs, count_args, op_left, op_right};
use patterns::pattern_report;

/// What a report needs around its error.
pub struct ReportContext<'a> {
    pub source: &'a Source<'a>,
    pub compat: Compat,
    pub trim: &'a dyn Fn(TextRange) -> TextRange,
    pub localizer: &'a Localizer,
}

impl std::fmt::Debug for ReportContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReportContext")
            .field("compat", &self.compat)
            .finish_non_exhaustive()
    }
}

/// `toReport`.
pub fn to_problem(error: &TypeError, cx: &ReportContext<'_>) -> Problem {
    let (title, range, message) = report(error, cx);
    let range = (cx.trim)(range);
    let region = cx.source.region(range, cx.compat);
    Problem::new(title, region, code::expand(&message, cx.source, cx.compat))
}

fn report(error: &TypeError, cx: &ReportContext<'_>) -> (&'static str, TextRange, Doc) {
    match error {
        TypeError::BadExpr(region, category, actual, expected) => {
            expr_report(cx, *region, category, actual, expected)
        }
        TypeError::BadPattern(region, category, actual, expected) => {
            pattern_report(cx, *region, category, actual, expected)
        }
        TypeError::InfiniteType(region, name, overall) => {
            records::infinite(cx, *region, name, overall)
        }
    }
}

/// `Code.toSnippet`: the code at `region`, `highlight` underlined.
fn snip(
    cx: &ReportContext<'_>,
    region: TextRange,
    highlight: Option<TextRange>,
    before: Doc,
    after: Doc,
) -> Doc {
    snippet(before, (cx.trim)(region), highlight.map(cx.trim), after)
}

fn yellow(doc: Doc) -> Doc {
    doc.color(Color::DullYellow)
}

fn green(doc: Doc) -> Doc {
    doc.color(Color::Green)
}

fn words(s: &str) -> Vec<Doc> {
    s.split(' ').filter(|w| !w.is_empty()).map(text).collect()
}

/// `D.toFancyHint`.
fn fancy_hint(docs: Vec<Doc>) -> Doc {
    fill_sep(std::iter::once(text("Hint").underline().append(text(":"))).chain(docs))
}

/// `D.fancyLink`.
fn fancy_link(label: &str, before: Vec<Doc>, page: &str, after: Vec<Doc>) -> Doc {
    let label = text(label).underline().append(text(":"));
    fill_sep(
        std::iter::once(label)
            .chain(before)
            .chain([Doc::link(page)])
            .chain(after),
    )
}

/// `D.link`.
fn link(label: &str, before: &str, page: &str, after: &str) -> Doc {
    nomi_base::diagnostic::doc::link(label, before, page, after)
}

/// `D.reflowLink`.
fn reflow_link(before: &str, page: &str, after: &str) -> Doc {
    nomi_base::diagnostic::doc::reflow_link(before, page, after)
}

/// `Suggest.sort` of `candidates` against `target`.
fn nearest(target: &str, mut candidates: Vec<String>) -> Vec<String> {
    let codes = |s: &str| s.chars().map(u32::from).collect::<Vec<u32>>();
    suggest::sort(&codes(target), &mut candidates, |c| codes(c));
    candidates
}

/// `typeComparison`.
fn type_comparison(
    localizer: &Localizer,
    actual: &t::Type,
    expected: &t::Type,
    i_am_seeing: &str,
    instead_of: &str,
    context_hints: Vec<Doc>,
) -> Doc {
    let (actual_doc, expected_doc, problems) = to_comparison(localizer, actual, expected);
    let mut docs = vec![
        reflow(i_am_seeing),
        actual_doc.indent(4),
        reflow(instead_of),
        expected_doc.indent(4),
    ];
    docs.extend(context_hints);
    docs.extend(problems_to_hint(&problems));
    stack(docs)
}

/// `loneType`.
fn lone_type(
    localizer: &Localizer,
    actual: &t::Type,
    expected: &t::Type,
    i_am_seeing: Doc,
    further: Vec<Doc>,
) -> Doc {
    let (actual_doc, _, problems) = to_comparison(localizer, actual, expected);
    let mut docs = vec![i_am_seeing, actual_doc.indent(4)];
    docs.extend(further);
    docs.extend(problems_to_hint(&problems));
    stack(docs)
}

/// `addCategory`.
fn add_category(this_is: &str, category: &Category) -> String {
    match category {
        Category::Local(name) | Category::Foreign(name) => format!("This `{name}` value is a:"),
        Category::Access(field) => format!("The value at .{field} is a:"),
        Category::Accessor(field) => format!("This .{field} field access function has type:"),
        Category::If => "This `if` expression produces:".to_owned(),
        Category::Case => "This `case` expression produces:".to_owned(),
        Category::List => format!("{this_is} a list of type:"),
        Category::Number => format!("{this_is} a number of type:"),
        Category::Float => format!("{this_is} a float of type:"),
        Category::String => format!("{this_is} a string of type:"),
        Category::Char => format!("{this_is} a character of type:"),
        Category::Lambda => format!("{this_is} an anonymous function of type:"),
        Category::Record => format!("{this_is} a record of type:"),
        Category::Tuple => format!("{this_is} a tuple of type:"),
        Category::Unit => format!("{this_is} a unit value:"),
        Category::Shader => format!("{this_is} a GLSL shader of type:"),
        Category::Effects => format!("{this_is} a thing for CORE LIBRARIES ONLY."),
        Category::CallResult(name) => match name {
            MaybeName::NoName | MaybeName::OpName(_) => format!("{this_is}:"),
            MaybeName::FuncName(name) | MaybeName::CtorName(name) => {
                format!("This `{name}` call produces:")
            }
        },
    }
}

/// `toExprReport`.
fn expr_report(
    cx: &ReportContext<'_>,
    expr_region: TextRange,
    category: &Category,
    actual: &t::Type,
    expected: &Expected<t::Type>,
) -> (&'static str, TextRange, Doc) {
    let localizer = cx.localizer;
    match expected {
        Expected::NoExpectation(expected) => {
            let comparison = type_comparison(
                localizer,
                actual,
                expected,
                &add_category("It is", category),
                "But you are trying to use it as:",
                Vec::new(),
            );
            let doc = snip(
                cx,
                expr_region,
                None,
                reflow("This expression is being used in an unexpected way:"),
                comparison,
            );
            ("TYPE MISMATCH", expr_region, doc)
        }
        Expected::FromAnnotation(name, _, sub, expected) => {
            let (thing, it_is) = match sub {
                SubContext::TypedIfBranch(i) => (
                    format!("{} branch of this `if` expression:", ordinal(i + 1)),
                    format!("The {} branch is", ordinal(i + 1)),
                ),
                SubContext::TypedCaseBranch(i) => (
                    format!("{} branch of this `case` expression:", ordinal(i + 1)),
                    format!("The {} branch is", ordinal(i + 1)),
                ),
                SubContext::TypedBody => (
                    format!("body of the `{name}` definition:"),
                    "The body is".to_owned(),
                ),
            };
            let comparison = type_comparison(
                localizer,
                actual,
                expected,
                &add_category(&it_is, category),
                &format!("But the type annotation on `{name}` says it should be:"),
                Vec::new(),
            );
            let doc = snip(
                cx,
                expr_region,
                None,
                reflow(&format!("Something is off with the {thing}")),
                comparison,
            );
            ("TYPE MISMATCH", expr_region, doc)
        }
        Expected::FromContext(region, context, expected) => from_context(
            cx,
            expr_region,
            *region,
            context,
            category,
            actual,
            expected,
        ),
    }
}

/// `toExprReport`'s `FromContext` cases.
fn from_context(
    cx: &ReportContext<'_>,
    expr_region: TextRange,
    region: TextRange,
    context: &Context,
    category: &Category,
    actual: &t::Type,
    expected: &t::Type,
) -> (&'static str, TextRange, Doc) {
    let localizer = cx.localizer;
    let mismatch = |highlight: Option<TextRange>,
                    problem: &str,
                    this_is: &str,
                    instead_of: &str,
                    further: Vec<Doc>| {
        let comparison = type_comparison(
            localizer,
            actual,
            expected,
            &add_category(this_is, category),
            instead_of,
            further,
        );
        (
            "TYPE MISMATCH",
            expr_region,
            snip(cx, region, highlight, reflow(problem), comparison),
        )
    };
    let bad_type =
        |highlight: Option<TextRange>, problem: &str, this_is: &str, further: Vec<Doc>| {
            let lone = lone_type(
                localizer,
                actual,
                expected,
                reflow(&add_category(this_is, category)),
                further,
            );
            (
                "TYPE MISMATCH",
                expr_region,
                snip(cx, region, highlight, reflow(problem), lone),
            )
        };
    match context {
        Context::ListEntry(index) => {
            let ith = ordinal(index + 1);
            mismatch(
                Some(expr_region),
                &format!(
                    "The {ith} element of this list does not match all the previous elements:"
                ),
                &format!("The {ith} element is"),
                "But all the previous elements in the list are:",
                vec![link(
                    "Hint",
                    "Everything in a list must be the same type of value. This way, we never run into unexpected values partway through a List.map, List.foldl, etc. Read",
                    "custom-types",
                    "to learn how to “mix” types.",
                )],
            )
        }
        Context::Negate => bad_type(
            Some(expr_region),
            "I do not know how to negate this type of value:",
            "It is",
            vec![fill_sep([
                text("But"),
                text("I"),
                text("only"),
                text("now"),
                text("how"),
                text("to"),
                text("negate"),
                yellow(text("Int")),
                text("and"),
                yellow(text("Float")),
                text("values."),
            ])],
        ),
        Context::CallArg(name, index) => {
            let ith = ordinal(index + 1);
            let this_function = match name {
                MaybeName::NoName => "this function".to_owned(),
                MaybeName::FuncName(n) | MaybeName::CtorName(n) => format!("`{n}`"),
                MaybeName::OpName(op) => format!("({op})"),
            };
            let further = if *index == 0 {
                Vec::new()
            } else {
                vec![simple_hint(
                    "I always figure out the argument types from left to right. If an argument is acceptable, I assume it is “correct” and move on. So the problem may actually be in one of the previous arguments!",
                )]
            };
            mismatch(
                Some(expr_region),
                &format!("The {ith} argument to {this_function} is not what I expect:"),
                "This argument is",
                &format!("But {this_function} needs the {ith} argument to be:"),
                further,
            )
        }
        Context::RecordUpdateValue(field) => mismatch(
            Some(expr_region),
            &format!("I cannot update the `{field}` field like this:"),
            &format!("You are trying to update `{field}` to be"),
            "But it should be:",
            vec![simple_note(
                "The record update syntax does not allow you to change the type of fields. You can achieve that with record constructors or the record literal syntax.",
            )],
        ),
        Context::Destructure => mismatch(
            None,
            "This definition is causing issues:",
            "You are defining",
            "But then trying to destructure it as:",
            Vec::new(),
        ),
        Context::OpLeft(op) => {
            let (problem, details) = op_left(localizer, category, op, actual, expected);
            (
                "TYPE MISMATCH",
                expr_region,
                snip(cx, region, Some(expr_region), problem, details),
            )
        }
        Context::OpRight(op) => {
            let (highlight, (problem, details)) =
                match op_right(localizer, category, op, actual, expected) {
                    RightDocs::Both(docs) => (None, docs),
                    RightDocs::Right(docs) => (Some(expr_region), docs),
                };
            (
                "TYPE MISMATCH",
                expr_region,
                snip(cx, region, highlight, problem, details),
            )
        }
        Context::CallArity(name, given) => {
            let missing = reflow("Are there any missing commas? Or missing parentheses?");
            let problem = match count_args(actual) {
                0 => {
                    let this_value = match name {
                        MaybeName::NoName => "This value".to_owned(),
                        MaybeName::FuncName(n) | MaybeName::CtorName(n) => {
                            format!("The `{n}` value")
                        }
                        MaybeName::OpName(op) => format!("The ({op}) operator"),
                    };
                    reflow(&format!(
                        "{this_value} is not a function, but it was given {}.",
                        args(*given)
                    ))
                }
                n => {
                    let this_function = match name {
                        MaybeName::NoName => "This function".to_owned(),
                        MaybeName::FuncName(n) => format!("The `{n}` function"),
                        MaybeName::CtorName(n) => format!("The `{n}` constructor"),
                        MaybeName::OpName(op) => format!("The ({op}) operator"),
                    };
                    reflow(&format!(
                        "{this_function} expects {}, but it got {given} instead.",
                        args(n)
                    ))
                }
            };
            (
                "TOO MANY ARGS",
                expr_region,
                snip(cx, region, Some(expr_region), problem, missing),
            )
        }
        Context::IfCondition => {
            let mut need = words("But I need this `if` condition to be a");
            need.extend([yellow(text("Bool")), text("value.")]);
            bad_type(
                Some(expr_region),
                "This `if` condition does not evaluate to a boolean value, True or False.",
                "It is",
                vec![fill_sep(need)],
            )
        }
        Context::IfBranch(index) => {
            let ith = ordinal(index + 1);
            mismatch(
                Some(expr_region),
                &format!("The {ith} branch of this `if` does not match all the previous branches:"),
                &format!("The {ith} branch is"),
                "But all the previous branches result in:",
                vec![link(
                    "Hint",
                    "All branches in an `if` must produce the same type of values. This way, no matter which branch we take, the result is always a consistent shape. Read",
                    "custom-types",
                    "to learn how to “mix” types.",
                )],
            )
        }
        Context::CaseBranch(index) => {
            let ith = ordinal(index + 1);
            mismatch(
                Some(expr_region),
                &format!(
                    "The {ith} branch of this `case` does not match all the previous branches:"
                ),
                &format!("The {ith} branch is"),
                "But all the previous branches result in:",
                vec![link(
                    "Hint",
                    "All branches in a `case` must produce the same type of values. This way, no matter which branch we take, the result is always a consistent shape. Read",
                    "custom-types",
                    "to learn how to “mix” types.",
                )],
            )
        }
        Context::RecordAccess(record_region, name, field_region, field) => records::record_access(
            cx,
            expr_region,
            region,
            *record_region,
            name.as_deref(),
            *field_region,
            field,
            category,
            actual,
            expected,
        ),
        Context::RecordUpdateKeys(record, fields) => records::record_update_keys(
            cx,
            expr_region,
            region,
            record,
            fields,
            category,
            actual,
            expected,
        ),
    }
}
