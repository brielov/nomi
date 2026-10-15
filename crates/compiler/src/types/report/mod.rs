//! The reports of type errors (`Reporting/Error/Type.hs`): annotations
//! and mismatches here, with the hints, operators, patterns, and records
//! and infinite types beside.

mod hints;
mod operators;
mod patterns;
mod records;

use nomi_base::TextRange;
use nomi_base::diagnostic::Problem;
use nomi_base::diagnostic::doc::{
    Doc, args, dull_yellow as yellow, fill_sep, link, ordinal, reflow, simple_hint, simple_note,
    stack, text, words,
};

use super::error::{self as t, Comparison, to_comparison};
use super::expected::{Category, Context, Expected, MaybeName, SubContext};
use super::localizer::Localizer;
use super::solve::TypeError;
use crate::report::{Report, ReportContext};
use hints::problems_to_hint;
use operators::{Explanation, RightDocs, op_left, op_right};

/// `toReport`.
pub fn to_problem(error: &TypeError, cx: &ReportContext<'_>, localizer: &Localizer) -> Problem {
    let report = match error {
        TypeError::BadExpr {
            region,
            category,
            actual,
            expected,
        } => expr_report(cx, localizer, *region, category, actual, expected),
        TypeError::BadPattern {
            region,
            category,
            actual,
            expected,
        } => patterns::pattern_report(cx, localizer, *region, category, actual, expected),
        TypeError::InfiniteType { region, name, ty } => {
            records::infinite(cx, localizer, *region, name, ty)
        }
    };
    cx.problem(&report)
}

/// Where a comparison's hints go: the context's before the problem's, as
/// `typeComparison` has them, or after, as `patternTypeComparison` does.
#[derive(Clone, Copy)]
enum HintOrder {
    ContextFirst,
    ProblemFirst,
}

/// `typeComparison`: what `i_am_seeing` is, and what it is `instead_of`,
/// then the hints.
fn type_comparison(
    localizer: &Localizer,
    actual: &t::Type,
    expected: &t::Type,
    i_am_seeing: &str,
    instead_of: &str,
    context_hints: Vec<Doc>,
    order: HintOrder,
) -> Doc {
    let Comparison {
        actual,
        expected,
        problems,
    } = to_comparison(localizer, actual, expected);
    let problem_hints = problems_to_hint(&problems);
    let hints = match order {
        HintOrder::ContextFirst => [context_hints, problem_hints],
        HintOrder::ProblemFirst => [problem_hints, context_hints],
    };
    let types = [
        reflow(i_am_seeing),
        actual.indent(4),
        reflow(instead_of),
        expected.indent(4),
    ];
    stack(types.into_iter().chain(hints.into_iter().flatten()))
}

/// `loneType`.
fn lone_type(
    localizer: &Localizer,
    actual: &t::Type,
    expected: &t::Type,
    i_am_seeing: Doc,
    further: Vec<Doc>,
) -> Doc {
    let comparison = to_comparison(localizer, actual, expected);
    let hints = problems_to_hint(&comparison.problems);
    stack(
        [i_am_seeing, comparison.actual.indent(4)]
            .into_iter()
            .chain(further)
            .chain(hints),
    )
}

/// The `Int and Float values.` that ends what arithmetic works with.
fn int_and_float_values() -> [Doc; 4] {
    [yellow("Int"), text("and"), yellow("Float"), text("values.")]
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

/// A type mismatch's two types, the expression's and the one it is
/// expected to have, with where and what the expression is.
struct Mismatch<'a> {
    /// The expression's range.
    expr_region: TextRange,
    /// The range of what expected the type.
    region: TextRange,
    category: &'a Category,
    actual: &'a t::Type,
    expected: &'a t::Type,
}

/// `toExprReport`.
fn expr_report(
    cx: &ReportContext<'_>,
    localizer: &Localizer,
    expr_region: TextRange,
    category: &Category,
    actual: &t::Type,
    expected: &Expected<t::Type>,
) -> Report {
    let compare = |expected, this_is: &str, instead_of: &str| {
        type_comparison(
            localizer,
            actual,
            expected,
            &add_category(this_is, category),
            instead_of,
            Vec::new(),
            HintOrder::ContextFirst,
        )
    };
    match expected {
        Expected::NoExpectation(expected) => {
            let comparison = compare(expected, "It is", "But you are trying to use it as:");
            let problem = reflow("This expression is being used in an unexpected way:");
            let doc = cx.snippet(expr_region, None, problem, comparison);
            Report::mismatch(expr_region, doc)
        }
        Expected::FromAnnotation {
            name,
            sub,
            ty: expected,
        } => {
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
            let instead_of = format!("But the type annotation on `{name}` says it should be:");
            let comparison = compare(expected, &it_is, &instead_of);
            let problem = reflow(&format!("Something is off with the {thing}"));
            let doc = cx.snippet(expr_region, None, problem, comparison);
            Report::mismatch(expr_region, doc)
        }
        Expected::FromContext {
            region,
            context,
            ty: expected,
        } => {
            let mismatch = Mismatch {
                expr_region,
                region: *region,
                category,
                actual,
                expected,
            };
            from_context(cx, localizer, &mismatch, context)
        }
    }
}

/// The hint that a list's or branches' values must be one type.
fn mix_types(explanation: &str) -> Doc {
    link(
        "Hint",
        explanation,
        "custom-types",
        "to learn how to “mix” types.",
    )
}

/// `toExprReport`'s `FromContext` cases.
#[expect(
    clippy::too_many_lines,
    reason = "upstream's `toExprReport` cases, one message per context"
)]
fn from_context(
    cx: &ReportContext<'_>,
    localizer: &Localizer,
    m: &Mismatch<'_>,
    context: &Context,
) -> Report {
    let snip = |highlight, problem, details| cx.snippet(m.region, highlight, problem, details);
    let mismatch = |highlight: Option<TextRange>,
                    problem: &str,
                    this_is: &str,
                    instead_of: &str,
                    further: Vec<Doc>| {
        let comparison = type_comparison(
            localizer,
            m.actual,
            m.expected,
            &add_category(this_is, m.category),
            instead_of,
            further,
            HintOrder::ContextFirst,
        );
        Report::mismatch(m.expr_region, snip(highlight, reflow(problem), comparison))
    };
    let bad_type = |problem: &str, further: Vec<Doc>| {
        let seeing = reflow(&add_category("It is", m.category));
        let lone = lone_type(localizer, m.actual, m.expected, seeing, further);
        Report::mismatch(
            m.expr_region,
            snip(Some(m.expr_region), reflow(problem), lone),
        )
    };
    match context {
        Context::ListEntry(index) => {
            let ith = ordinal(index + 1);
            mismatch(
                Some(m.expr_region),
                &format!(
                    "The {ith} element of this list does not match all the previous elements:"
                ),
                &format!("The {ith} element is"),
                "But all the previous elements in the list are:",
                vec![mix_types(
                    "Everything in a list must be the same type of value. This way, we never run into unexpected values partway through a List.map, List.foldl, etc. Read",
                )],
            )
        }
        Context::Negate => {
            let only = words("But I only now how to negate").chain(int_and_float_values());
            bad_type(
                "I do not know how to negate this type of value:",
                vec![fill_sep(only)],
            )
        }
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
                Some(m.expr_region),
                &format!("The {ith} argument to {this_function} is not what I expect:"),
                "This argument is",
                &format!("But {this_function} needs the {ith} argument to be:"),
                further,
            )
        }
        Context::RecordUpdateValue(field) => mismatch(
            Some(m.expr_region),
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
            let Explanation { problem, details } =
                op_left(localizer, m.category, op, m.actual, m.expected);
            Report::mismatch(m.expr_region, snip(Some(m.expr_region), problem, details))
        }
        Context::OpRight(op) => {
            let (highlight, Explanation { problem, details }) =
                match op_right(localizer, m.category, op, m.actual, m.expected) {
                    RightDocs::Both(explanation) => (None, explanation),
                    RightDocs::Right(explanation) => (Some(m.expr_region), explanation),
                };
            Report::mismatch(m.expr_region, snip(highlight, problem, details))
        }
        Context::CallArity(name, given) => {
            let missing = reflow("Are there any missing commas? Or missing parentheses?");
            let problem = match m.actual.arity() {
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
            let doc = snip(Some(m.expr_region), problem, missing);
            Report::new("TOO MANY ARGS", m.expr_region, doc)
        }
        Context::IfCondition => {
            let need = (words("But I need this `if` condition to be a"))
                .chain([yellow("Bool"), text("value.")]);
            bad_type(
                "This `if` condition does not evaluate to a boolean value, True or False.",
                vec![fill_sep(need)],
            )
        }
        Context::IfBranch(index) => {
            let ith = ordinal(index + 1);
            mismatch(
                Some(m.expr_region),
                &format!("The {ith} branch of this `if` does not match all the previous branches:"),
                &format!("The {ith} branch is"),
                "But all the previous branches result in:",
                vec![mix_types(
                    "All branches in an `if` must produce the same type of values. This way, no matter which branch we take, the result is always a consistent shape. Read",
                )],
            )
        }
        Context::CaseBranch(index) => {
            let ith = ordinal(index + 1);
            mismatch(
                Some(m.expr_region),
                &format!(
                    "The {ith} branch of this `case` does not match all the previous branches:"
                ),
                &format!("The {ith} branch is"),
                "But all the previous branches result in:",
                vec![mix_types(
                    "All branches in a `case` must produce the same type of values. This way, no matter which branch we take, the result is always a consistent shape. Read",
                )],
            )
        }
        Context::RecordAccess {
            record,
            record_name,
            field_region,
            field,
        } => records::record_access(
            cx,
            localizer,
            m,
            *record,
            record_name.as_deref(),
            *field_region,
            field,
        ),
        Context::RecordUpdateKeys(record, fields) => {
            records::record_update_keys(cx, localizer, m, record, fields)
        }
    }
}
