//! The reports of type errors (`Reporting/Error/Type.hs`), ported in
//! parts: annotations and mismatches (6.11), calls and operators (6.12),
//! branches and patterns (6.13), records and infinite types (6.14). An
//! error of a part still to come is reported by its title at its region.

use nomi_base::diagnostic::Problem;
use nomi_base::diagnostic::code::{self, Source};
use nomi_base::diagnostic::doc::{
    Color, Doc, comma_sep, fill_sep, ordinal, reflow, simple_hint, simple_note, snippet, stack,
    text,
};
use nomi_base::{Compat, TextRange, suggest};

use super::error::{self as t, Direction, Problem as TProblem, Super, to_comparison};
use super::expected::{Category, Context, Expected, MaybeName, SubContext};
use super::localizer::Localizer;
use super::solve::TypeError;

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
        TypeError::BadPattern(region, ..) => placeholder("TYPE MISMATCH", *region),
        TypeError::InfiniteType(region, ..) => placeholder("INFINITE TYPE", *region),
    }
}

/// A report still to port: its title and region.
fn placeholder(title: &'static str, region: TextRange) -> (&'static str, TextRange, Doc) {
    (
        title,
        region,
        reflow("nomi finds this type error but doesn't explain it yet."),
    )
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

/// `problemsToHint`: the first problem's.
fn problems_to_hint(problems: &[TProblem]) -> Vec<Doc> {
    problems.first().map(problem_to_hint).unwrap_or_default()
}

/// `problemToHint`.
fn problem_to_hint(problem: &TProblem) -> Vec<Doc> {
    let fancy = |s: &[&str], green_word: &str, rest: &[&str]| {
        let mut docs: Vec<Doc> = s.iter().map(|w| text(*w)).collect();
        docs.push(green(text(green_word.to_owned())));
        docs.extend(rest.iter().map(|w| text(*w)));
        vec![fancy_hint(docs)]
    };
    match problem {
        TProblem::IntFloat => {
            let mut after =
                words("to learn why Elm does not implicitly convert Ints to Floats. Use");
            after.push(green(text("toFloat")));
            after.push(text("and"));
            after.push(green(text("round")));
            after.extend(words("to do explicit conversions."));
            vec![fancy_link(
                "Note",
                vec![text("Read")],
                "implicit-casts",
                after,
            )]
        }
        TProblem::StringFromInt => fancy(
            &[
                "Want", "to", "convert", "an", "Int", "into", "a", "String?", "Use", "the",
            ],
            "String.fromInt",
            &["function!"],
        ),
        TProblem::StringFromFloat => fancy(
            &[
                "Want", "to", "convert", "a", "Float", "into", "a", "String?", "Use", "the",
            ],
            "String.fromFloat",
            &["function!"],
        ),
        TProblem::StringToInt => fancy(
            &[
                "Want", "to", "convert", "a", "String", "into", "an", "Int?", "Use", "the",
            ],
            "String.toInt",
            &["function!"],
        ),
        TProblem::StringToFloat => fancy(
            &[
                "Want", "to", "convert", "a", "String", "into", "a", "Float?", "Use", "the",
            ],
            "String.toFloat",
            &["function!"],
        ),
        TProblem::AnythingToBool => vec![simple_hint(
            "Elm does not have “truthiness” such that ints and strings and lists are automatically converted to booleans. Do that conversion explicitly!",
        )],
        TProblem::AnythingFromMaybe => fancy(
            &["Use"],
            "Maybe.withDefault",
            &[
                "to", "handle", "possible", "errors.", "Longer", "term,", "it", "is", "usually",
                "better", "to", "write", "out", "the", "full", "`case`", "though!",
            ],
        ),
        TProblem::ArityMismatch(x, y) => vec![simple_hint(&if x < y {
            format!(
                "It looks like it takes too few arguments. I was expecting {} more.",
                y - x
            )
        } else {
            format!(
                "It looks like it takes too many arguments. I see {} extra.",
                x - y
            )
        })],
        TProblem::BadFlexSuper(direction, s, _, ty) => match ty {
            t::Type::Infinite | t::Type::Error | t::Type::FlexVar(_) => Vec::new(),
            t::Type::FlexSuper(s2, _) => bad_flex_flex_super(*s, *s2),
            t::Type::RigidVar(y) => bad_rigid_var(y, to_a_super_thing(*s)),
            t::Type::RigidSuper(s2, _) => bad_rigid_super(*s2, to_a_super_thing(*s)),
            _ => bad_flex_super(*direction, *s, ty),
        },
        TProblem::BadRigidVar(x, ty) => match ty {
            t::Type::Lambda(..) => bad_rigid_var(x, "a function"),
            t::Type::Infinite | t::Type::Error | t::Type::FlexVar(_) => Vec::new(),
            t::Type::FlexSuper(s, _) => bad_rigid_var(x, to_a_super_thing(*s)),
            t::Type::RigidVar(y) | t::Type::RigidSuper(_, y) => bad_double_rigid(x, y),
            t::Type::Type(_, n, _) | t::Type::Alias(_, n, _, _) => {
                bad_rigid_var(x, &format!("a `{n}` value"))
            }
            t::Type::Record(..) => bad_rigid_var(x, "a record"),
            t::Type::Unit => bad_rigid_var(x, "a unit value"),
            t::Type::Tuple(..) => bad_rigid_var(x, "a tuple"),
        },
        TProblem::BadRigidSuper(s, x, ty) => match ty {
            t::Type::Lambda(..) => bad_rigid_super(*s, "a function"),
            t::Type::Infinite | t::Type::Error | t::Type::FlexVar(_) => Vec::new(),
            t::Type::FlexSuper(s2, _) => bad_rigid_super(*s, to_a_super_thing(*s2)),
            t::Type::RigidVar(y) | t::Type::RigidSuper(_, y) => bad_double_rigid(x, y),
            t::Type::Type(_, n, _) | t::Type::Alias(_, n, _, _) => {
                bad_rigid_super(*s, &format!("a `{n}` value"))
            }
            t::Type::Record(..) => bad_rigid_super(*s, "a record"),
            t::Type::Unit => bad_rigid_super(*s, "a unit value"),
            t::Type::Tuple(..) => bad_rigid_super(*s, "a tuple"),
        },
        TProblem::FieldsMissing(fields) => {
            let docs: Vec<Doc> = fields.iter().map(|f| green(text(f.clone()))).collect();
            match &docs[..] {
                [] => Vec::new(),
                [f1] => {
                    let mut hint = words("Looks like the");
                    hint.push(f1.clone());
                    hint.extend(words("field is missing."));
                    vec![fancy_hint(hint)]
                }
                _ => {
                    let mut hint = words("Looks like fields");
                    hint.extend(comma_sep("and", docs));
                    hint.extend(words("are missing."));
                    vec![fancy_hint(hint)]
                }
            }
        }
        TProblem::FieldTypo(typo, possibilities) => {
            match nearest(typo, possibilities.clone()).first() {
                None => Vec::new(),
                Some(nearest) => {
                    let mut hint = words("Seems like a record field typo. Maybe");
                    hint.push(yellow(text(typo.clone())));
                    hint.extend(words("should be"));
                    hint.push(green(text(nearest.clone())).append(text("?")));
                    vec![
                        fancy_hint(hint),
                        simple_hint(
                            "Can more type annotations be added? Type annotations always help me give more specific messages, and I think they could help a lot in this case!",
                        ),
                    ]
                }
            }
        }
    }
}

/// `badRigidVar`.
fn bad_rigid_var(name: &str, a_thing: &str) -> Vec<Doc> {
    vec![
        simple_hint(&format!(
            "Your type annotation uses type variable `{name}` which means ANY type of value can flow through, but your code is saying it specifically wants {a_thing}. Maybe change your type annotation to be more specific? Maybe change the code to be more general?"
        )),
        reflow_link("Read", "type-annotations", "for more advice!"),
    ]
}

/// `badDoubleRigid`.
fn bad_double_rigid(x: &str, y: &str) -> Vec<Doc> {
    vec![
        simple_hint(&format!(
            "Your type annotation uses `{x}` and `{y}` as separate type variables. Your code seems to be saying they are the same though. Maybe they should be the same in your type annotation? Maybe your code uses them in a weird way?"
        )),
        reflow_link("Read", "type-annotations", "for more advice!"),
    ]
}

/// `toASuperThing`.
fn to_a_super_thing(s: Super) -> &'static str {
    match s {
        Super::Number => "a `number` value",
        Super::Comparable => "a `comparable` value",
        Super::CompAppend => "a `compappend` value",
        Super::Appendable => "an `appendable` value",
    }
}

/// `badFlexSuper`.
fn bad_flex_super(direction: Direction, s: Super, ty: &t::Type) -> Vec<Doc> {
    match s {
        Super::Comparable => match ty {
            t::Type::Record(..) => vec![link(
                "Hint",
                "I do not know how to compare records. I can only compare ints, floats, chars, strings, lists of comparable values, and tuples of comparable values. Check out",
                "comparing-records",
                "for ideas on how to proceed.",
            )],
            t::Type::Type(_, name, _) => vec![
                simple_hint(&format!(
                    "I do not know how to compare `{name}` values. I can only compare ints, floats, chars, strings, lists of comparable values, and tuples of comparable values."
                )),
                reflow_link(
                    "Check out",
                    "comparing-custom-types",
                    "for ideas on how to proceed.",
                ),
            ],
            _ => vec![simple_hint(
                "I only know how to compare ints, floats, chars, strings, lists of comparable values, and tuples of comparable values.",
            )],
        },
        Super::Appendable => vec![simple_hint("I only know how to append strings and lists.")],
        Super::CompAppend => vec![simple_hint(
            "Only strings and lists are both comparable and appendable.",
        )],
        Super::Number => match ty {
            t::Type::Type(home, name, _) if t::is_string(home, name) => match direction {
                Direction::Have => {
                    let mut docs = words("Try using");
                    docs.push(green(text("String.fromInt")));
                    docs.extend(words("to convert it to a string?"));
                    vec![fancy_hint(docs)]
                }
                Direction::Need => {
                    let mut docs = words("Try using");
                    docs.push(green(text("String.toInt")));
                    docs.extend(words("to convert it to an integer?"));
                    vec![fancy_hint(docs)]
                }
            },
            _ => vec![fancy_hint(vec![
                text("Only"),
                green(text("Int")),
                text("and"),
                green(text("Float")),
                text("values"),
                text("work"),
                text("as"),
                text("numbers."),
            ])],
        },
    }
}

/// `badRigidSuper`.
fn bad_rigid_super(s: Super, a_thing: &str) -> Vec<Doc> {
    let (super_type, many_things) = match s {
        Super::Number => ("number", "ints AND floats"),
        Super::Comparable => (
            "comparable",
            "ints, floats, chars, strings, lists, and tuples",
        ),
        Super::Appendable => ("appendable", "strings AND lists"),
        Super::CompAppend => ("compappend", "strings AND lists"),
    };
    vec![
        simple_hint(&format!(
            "The `{super_type}` in your type annotation is saying that {many_things} can flow through, but your code is saying it specifically wants {a_thing}. Maybe change your type annotation to be more specific? Maybe change the code to be more general?"
        )),
        reflow_link("Read", "type-annotations", "for more advice!"),
    ]
}

/// `badFlexFlexSuper`.
fn bad_flex_flex_super(s1: Super, s2: Super) -> Vec<Doc> {
    let like_this = |s| match s {
        Super::Number => "a number",
        Super::Comparable => "comparable",
        Super::CompAppend => "a compappend",
        Super::Appendable => "appendable",
    };
    vec![simple_hint(&format!(
        "There are no values in Elm that are both {} and {}.",
        like_this(s1),
        like_this(s2)
    ))]
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
        Context::OpLeft(_)
        | Context::OpRight(_)
        | Context::CallArity(..)
        | Context::IfCondition
        | Context::IfBranch(_)
        | Context::CaseBranch(_)
        | Context::RecordAccess(..)
        | Context::RecordUpdateKeys(..) => placeholder("TYPE MISMATCH", expr_region),
    }
}
