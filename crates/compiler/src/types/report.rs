//! The reports of type errors (`Reporting/Error/Type.hs`), ported in
//! parts: annotations and mismatches (6.11), calls and operators (6.12),
//! branches and patterns (6.13), records and infinite types (6.14). An
//! error of a part still to come is reported by its title at its region.

use nomi_base::diagnostic::Problem;
use nomi_base::diagnostic::code::{self, Source};
use nomi_base::diagnostic::doc::{
    Color, Doc, args, comma_sep, fill_sep, ordinal, reflow, simple_hint, simple_note, snippet,
    stack, text, vcat,
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
        Context::IfCondition
        | Context::IfBranch(_)
        | Context::CaseBranch(_)
        | Context::RecordAccess(..)
        | Context::RecordUpdateKeys(..) => placeholder("TYPE MISMATCH", expr_region),
    }
}

/// `countArgs`.
fn count_args(ty: &t::Type) -> usize {
    match ty {
        t::Type::Lambda(_, _, rest) => 1 + rest.len(),
        _ => 0,
    }
}

fn atom(ty: &t::Type, test: fn(&crate::canonical::Home, &str) -> bool) -> bool {
    matches!(ty, t::Type::Type(home, name, args) if args.is_empty() && test(home, name))
}

fn is_int(ty: &t::Type) -> bool {
    atom(ty, t::is_int)
}

fn is_float(ty: &t::Type) -> bool {
    atom(ty, t::is_float)
}

fn is_string(ty: &t::Type) -> bool {
    atom(ty, t::is_string)
}

fn is_list(ty: &t::Type) -> bool {
    matches!(ty, t::Type::Type(home, name, args) if args.len() == 1 && t::is_list(home, name))
}

/// `opLeftToDocs`.
fn op_left(
    localizer: &Localizer,
    category: &Category,
    op: &str,
    ty: &t::Type,
    expected: &t::Type,
) -> (Doc, Doc) {
    match op {
        "+" if is_string(ty) => bad_string_add(),
        "+" if is_list(ty) => bad_list_add(localizer, category, "left", ty, expected),
        "+" => bad_math(
            localizer,
            category,
            "Addition",
            "left",
            "+",
            ty,
            expected,
            Vec::new(),
        ),
        "*" if is_list(ty) => bad_list_mul(localizer, category, "left", ty, expected),
        "*" => bad_math(
            localizer,
            category,
            "Multiplication",
            "left",
            "*",
            ty,
            expected,
            Vec::new(),
        ),
        "-" => bad_math(
            localizer,
            category,
            "Subtraction",
            "left",
            "-",
            ty,
            expected,
            Vec::new(),
        ),
        "^" => bad_math(
            localizer,
            category,
            "Exponentiation",
            "left",
            "^",
            ty,
            expected,
            Vec::new(),
        ),
        "/" => bad_f_div(localizer, "left", ty, expected),
        "//" => bad_i_div(localizer, "left", ty, expected),
        "&&" | "||" => bad_bool(localizer, op, "left", ty, expected),
        "<" | ">" | "<=" | ">=" => bad_comp_left(localizer, category, op, "left", ty, expected),
        "++" => bad_append_left(localizer, category, ty, expected),
        "<|" => (
            reflow("The left side of (<|) needs to be a function so I can pipe arguments to it!"),
            lone_type(
                localizer,
                ty,
                expected,
                reflow(&add_category("I am seeing", category)),
                vec![reflow("This needs to be some kind of function though!")],
            ),
        ),
        _ => (
            reflow(&format!("The left argument of ({op}) is causing problems:")),
            type_comparison(
                localizer,
                ty,
                expected,
                &add_category("The left argument is", category),
                &format!("But ({op}) needs the left argument to be:"),
                Vec::new(),
            ),
        ),
    }
}

/// `RightDocs`: whether a report highlights both operands or the right.
enum RightDocs {
    Both((Doc, Doc)),
    Right((Doc, Doc)),
}

/// `opRightToDocs`.
fn op_right(
    localizer: &Localizer,
    category: &Category,
    op: &str,
    ty: &t::Type,
    expected: &t::Type,
) -> RightDocs {
    let cast = if is_float(expected) && is_int(ty) {
        Some(true)
    } else if is_int(expected) && is_float(ty) {
        Some(false)
    } else {
        None
    };
    match op {
        "+" | "*" | "-" | "^" if cast.is_some() => bad_cast(op, cast == Some(true)),
        "+" if is_string(ty) => RightDocs::Right(bad_string_add()),
        "+" if is_list(ty) => {
            RightDocs::Right(bad_list_add(localizer, category, "right", ty, expected))
        }
        "+" => RightDocs::Right(bad_math(
            localizer,
            category,
            "Addition",
            "right",
            "+",
            ty,
            expected,
            Vec::new(),
        )),
        "*" if is_list(ty) => {
            RightDocs::Right(bad_list_mul(localizer, category, "right", ty, expected))
        }
        "*" => RightDocs::Right(bad_math(
            localizer,
            category,
            "Multiplication",
            "right",
            "*",
            ty,
            expected,
            Vec::new(),
        )),
        "-" => RightDocs::Right(bad_math(
            localizer,
            category,
            "Subtraction",
            "right",
            "-",
            ty,
            expected,
            Vec::new(),
        )),
        "^" => RightDocs::Right(bad_math(
            localizer,
            category,
            "Exponentiation",
            "right",
            "^",
            ty,
            expected,
            Vec::new(),
        )),
        "/" => RightDocs::Right(bad_f_div(localizer, "right", ty, expected)),
        "//" => RightDocs::Right(bad_i_div(localizer, "right", ty, expected)),
        "&&" | "||" => RightDocs::Right(bad_bool(localizer, op, "right", ty, expected)),
        "<" | ">" | "<=" | ">=" => bad_comp_right(localizer, op, ty, expected),
        "==" | "/=" => bad_equality(localizer, op, ty, expected),
        "::" => bad_cons_right(localizer, category, ty, expected),
        "++" => bad_append_right(localizer, category, ty, expected),
        "<|" => RightDocs::Right((
            reflow("I cannot send this through the (<|) pipe:"),
            type_comparison(
                localizer,
                ty,
                expected,
                "The argument is:",
                "But (<|) is piping it to a function that expects:",
                Vec::new(),
            ),
        )),
        "|>" => match (ty, expected) {
            (t::Type::Lambda(expected_arg, _, _), t::Type::Lambda(arg, _, _)) => {
                RightDocs::Right((
                    reflow("This function cannot handle the argument sent through the (|>) pipe:"),
                    type_comparison(
                        localizer,
                        arg,
                        expected_arg,
                        "The argument is:",
                        "But (|>) is piping it to a function that expects:",
                        Vec::new(),
                    ),
                ))
            }
            _ => RightDocs::Right((
                reflow(
                    "The right side of (|>) needs to be a function so I can pipe arguments to it!",
                ),
                lone_type(
                    localizer,
                    ty,
                    expected,
                    reflow(&add_category(
                        "But instead of a function, I am seeing",
                        category,
                    )),
                    Vec::new(),
                ),
            )),
        },
        _ => bad_op_right_fallback(localizer, category, op, ty, expected),
    }
}

/// `badOpRightFallback`.
fn bad_op_right_fallback(
    localizer: &Localizer,
    category: &Category,
    op: &str,
    ty: &t::Type,
    expected: &t::Type,
) -> RightDocs {
    RightDocs::Right((
        reflow(&format!(
            "The right argument of ({op}) is causing problems."
        )),
        type_comparison(
            localizer,
            ty,
            expected,
            &add_category("The right argument is", category),
            &format!("But ({op}) needs the right argument to be:"),
            vec![simple_hint(&format!(
                "With operators like ({op}) I always check the left side first. If it seems fine, I assume it is correct and check the right side. So the problem may be in how the left and right arguments interact!"
            ))],
        ),
    ))
}

/// `badConsRight`.
fn bad_cons_right(
    localizer: &Localizer,
    category: &Category,
    ty: &t::Type,
    expected: &t::Type,
) -> RightDocs {
    match (ty, expected) {
        (t::Type::Type(h1, n1, a1), t::Type::Type(h2, n2, a2))
            if a1.len() == 1 && t::is_list(h1, n1) && a2.len() == 1 && t::is_list(h2, n2) =>
        {
            let (actual_element, expected_element) = (&a1[0], &a2[0]);
            let hint = match expected_element {
                t::Type::Type(h, n, a) if a.len() == 1 && t::is_list(h, n) => simple_hint(
                    "Are you trying to append two lists? The (++) operator appends lists, whereas the (::) operator is only for adding ONE element to a list.",
                ),
                _ => reflow("Lists need ALL elements to be the same type though."),
            };
            RightDocs::Both((
                reflow("I am having trouble with this (::) operator:"),
                type_comparison(
                    localizer,
                    expected_element,
                    actual_element,
                    "The left side of (::) is:",
                    "But you are trying to put that into a list filled with:",
                    vec![hint],
                ),
            ))
        }
        (t::Type::Type(h1, n1, a1), _) if a1.len() == 1 && t::is_list(h1, n1) => {
            bad_op_right_fallback(localizer, category, "::", ty, expected)
        }
        _ => {
            let mut details = words("But (::) needs a");
            details.push(yellow(text("List")));
            details.extend(words("on the right."));
            RightDocs::Right((
                reflow("The (::) operator can only add elements onto lists."),
                lone_type(
                    localizer,
                    ty,
                    expected,
                    reflow(&add_category("The right side is", category)),
                    vec![fill_sep(details)],
                ),
            ))
        }
    }
}

/// `AppendType`.
enum AppendType {
    Number(&'static str, &'static str),
    String,
    List,
    Other,
}

/// `toAppendType`.
fn append_type(ty: &t::Type) -> AppendType {
    match ty {
        t::Type::Type(home, name, _) if t::is_int(home, name) => {
            AppendType::Number("Int", "String.fromInt")
        }
        t::Type::Type(home, name, _) if t::is_float(home, name) => {
            AppendType::Number("Float", "String.fromFloat")
        }
        t::Type::Type(home, name, _) if t::is_string(home, name) => AppendType::String,
        t::Type::Type(home, name, _) if t::is_list(home, name) => AppendType::List,
        t::Type::FlexSuper(Super::Number, _) => AppendType::Number("number", "String.fromInt"),
        _ => AppendType::Other,
    }
}

/// `badAppendLeft`.
fn bad_append_left(
    localizer: &Localizer,
    category: &Category,
    ty: &t::Type,
    expected: &t::Type,
) -> (Doc, Doc) {
    match append_type(ty) {
        AppendType::Number(thing, from) => {
            let mut first = words("The (++) operator can append List and String values, but not");
            first.push(yellow(text(thing)));
            first.extend(words("values like this:"));
            let mut second = words("Try using");
            second.push(green(text(from)));
            second.extend(words(
                "to turn it into a string? Or put it in [] to make it a list? Or switch to the (::) operator?",
            ));
            (fill_sep(first), fill_sep(second))
        }
        _ => {
            let mut details = words("But the (++) operator is only for appending");
            details.extend([
                yellow(text("List")),
                text("and"),
                yellow(text("String")),
                text("values."),
            ]);
            details.extend(words("Maybe put this value in [] to make it a list?"));
            (
                reflow("The (++) operator cannot append this type of value:"),
                lone_type(
                    localizer,
                    ty,
                    expected,
                    reflow(&add_category("I am seeing", category)),
                    vec![fill_sep(details)],
                ),
            )
        }
    }
}

/// `badAppendRight`.
fn bad_append_right(
    localizer: &Localizer,
    category: &Category,
    ty: &t::Type,
    expected: &t::Type,
) -> RightDocs {
    match (append_type(expected), append_type(ty)) {
        (AppendType::String, AppendType::Number(thing, from)) => {
            let mut first = words("I thought I was appending");
            first.push(yellow(text("String")));
            first.extend(words("values here, not"));
            first.push(yellow(text(thing)));
            first.extend(words("values like this:"));
            let mut second = words("Try using");
            second.push(green(text(from)));
            second.extend(words("to turn it into a string?"));
            RightDocs::Right((fill_sep(first), fill_sep(second)))
        }
        (AppendType::List, AppendType::Number(thing, _)) => {
            let mut first = words("I thought I was appending");
            first.push(yellow(text("List")));
            first.extend(words("values here, not"));
            first.push(yellow(text(thing)));
            first.extend(words("values like this:"));
            RightDocs::Right((
                fill_sep(first),
                reflow("Try putting it in [] to make it a list?"),
            ))
        }
        (AppendType::String, AppendType::List) => same_sides("String", "List"),
        (AppendType::List, AppendType::String) => same_sides("List", "String"),
        _ => RightDocs::Both((
            reflow("The (++) operator cannot append these two values:"),
            type_comparison(
                localizer,
                expected,
                ty,
                "I already figured out that the left side of (++) is:",
                &add_category("But this clashes with the right side, which is", category),
                Vec::new(),
            ),
        )),
    }
}

/// A String on one side of (++) and a List on the other.
fn same_sides(left: &str, right: &str) -> RightDocs {
    let mut details = words("I see a");
    details.push(yellow(text(left.to_owned())));
    details.extend(words("on the left and a"));
    details.push(yellow(text(right.to_owned())));
    details.extend(words(
        "on the right. Which should it be? Does the string need [] around it to become a list?",
    ));
    RightDocs::Both((
        reflow("The (++) operator needs the same type of value on both sides:"),
        fill_sep(details),
    ))
}

/// `badCast`: Int and Float on either side; `float_int` when the left is
/// the Float.
fn bad_cast(op: &str, float_int: bool) -> RightDocs {
    let an_int = vec![text("an"), yellow(text("Int"))];
    let a_float = vec![text("a"), yellow(text("Float"))];
    let (to_float, round) = (green(text("toFloat")), green(text("round")));
    let details = if float_int {
        cast_help(a_float, an_int, round, to_float)
    } else {
        cast_help(an_int, a_float, to_float, round)
    };
    RightDocs::Both((
        reflow(&format!(
            "I need both sides of ({op}) to be the exact same type. Both Int or both Float."
        )),
        details,
    ))
}

/// `badCastHelp`.
fn cast_help(left: Vec<Doc>, right: Vec<Doc>, fix_left: Doc, fix_right: Doc) -> Doc {
    let mut first = words("But I see");
    first.extend(left);
    first.extend(words("on the left and"));
    first.extend(right);
    first.extend(words("on the right."));
    let mut second = vec![text("Use"), fix_left];
    second.extend(words("on the left (or"));
    second.push(fix_right);
    second.extend(words("on the right) to make both sides match!"));
    stack([
        fill_sep(first),
        fill_sep(second),
        link(
            "Note",
            "Read",
            "implicit-casts",
            "to learn why Elm does not implicitly convert Ints to Floats.",
        ),
    ])
}

/// `badStringAdd`.
fn bad_string_add() -> (Doc, Doc) {
    let mut first = words("I cannot do addition with");
    first.push(yellow(text("String")));
    first.extend(words("values like this one:"));
    let mut works = words("The (+) operator only works with");
    works.extend([
        yellow(text("Int")),
        text("and"),
        yellow(text("Float")),
        text("values."),
    ]);
    let mut hint = words("Switch to the");
    hint.push(green(text("(++)")));
    hint.extend(words("operator to append strings!"));
    (fill_sep(first), stack([fill_sep(works), fancy_hint(hint)]))
}

/// `badListAdd`.
fn bad_list_add(
    localizer: &Localizer,
    category: &Category,
    direction: &str,
    ty: &t::Type,
    expected: &t::Type,
) -> (Doc, Doc) {
    let mut works = words("But (+) only works with");
    works.extend([
        yellow(text("Int")),
        text("and"),
        yellow(text("Float")),
        text("values."),
    ]);
    let mut hint = words("Switch to the");
    hint.push(green(text("(++)")));
    hint.extend(words("operator to append lists!"));
    (
        text("I cannot do addition with lists:"),
        lone_type(
            localizer,
            ty,
            expected,
            reflow(&add_category(
                &format!("The {direction} side of (+) is"),
                category,
            )),
            vec![fill_sep(works), fancy_hint(hint)],
        ),
    )
}

/// `badListMul`.
fn bad_list_mul(
    localizer: &Localizer,
    category: &Category,
    direction: &str,
    ty: &t::Type,
    expected: &t::Type,
) -> (Doc, Doc) {
    let mut hint = words("Maybe you want");
    hint.push(green(text("List.repeat")));
    hint.extend(words("to build a list of repeated values?"));
    bad_math(
        localizer,
        category,
        "Multiplication",
        direction,
        "*",
        ty,
        expected,
        vec![fancy_hint(hint)],
    )
}

/// `badMath`.
#[allow(clippy::too_many_arguments)]
fn bad_math(
    localizer: &Localizer,
    category: &Category,
    operation: &str,
    direction: &str,
    op: &str,
    ty: &t::Type,
    expected: &t::Type,
    other_hints: Vec<Doc>,
) -> (Doc, Doc) {
    let mut works = vec![
        text("But"),
        text(format!("({op})")),
        text("only"),
        text("works"),
        text("with"),
    ];
    works.extend([
        yellow(text("Int")),
        text("and"),
        yellow(text("Float")),
        text("values."),
    ]);
    let mut further = vec![fill_sep(works)];
    further.extend(other_hints);
    (
        reflow(&format!("{operation} does not work with this value:")),
        lone_type(
            localizer,
            ty,
            expected,
            reflow(&add_category(
                &format!("The {direction} side of ({op}) is"),
                category,
            )),
            further,
        ),
    )
}

/// The opening of `badFDiv` and `badIDiv`: `The left side of (op) must
/// be a Type,`.
fn must_be(direction: &str, op: &str, article: &str, ty: &str) -> Vec<Doc> {
    let mut docs = vec![text("The"), text(direction.to_owned())];
    docs.extend(words(&format!("side of ({op}) must be {article}")));
    docs.push(yellow(text(ty.to_owned())).append(text(",")));
    docs
}

/// `badFDiv`.
fn bad_f_div(
    localizer: &Localizer,
    direction: &str,
    ty: &t::Type,
    expected: &t::Type,
) -> (Doc, Doc) {
    let details = if is_int(ty) {
        let mut first = must_be(direction, "/", "a", "Float");
        first.extend(words("but I am seeing an"));
        first.push(yellow(text("Int")).append(text(".")));
        first.extend(words("I recommend:"));
        stack([
            fill_sep(first),
            vcat([
                green(text("toFloat"))
                    .append(text(" for explicit conversions     "))
                    .append(text("(toFloat 5 / 2) == 2.5").color(Color::Black)),
                green(text("(//)   "))
                    .append(text(" for integer division         "))
                    .append(text("(5 // 2)        == 2").color(Color::Black)),
            ]),
            link(
                "Note",
                "Read",
                "implicit-casts",
                "to learn why Elm does not implicitly convert Ints to Floats.",
            ),
        ])
    } else {
        let mut seeing = must_be(direction, "/", "a", "Float");
        seeing.extend(words("but instead I am seeing:"));
        lone_type(localizer, ty, expected, fill_sep(seeing), Vec::new())
    };
    (
        reflow("The (/) operator is specifically for floating-point division:"),
        details,
    )
}

/// `badIDiv`.
fn bad_i_div(
    localizer: &Localizer,
    direction: &str,
    ty: &t::Type,
    expected: &t::Type,
) -> (Doc, Doc) {
    let details = if is_float(ty) {
        let mut first = must_be(direction, "//", "an", "Int");
        first.extend(words("but I am seeing a"));
        first.push(yellow(text("Float")).append(text(".")));
        first.extend(words(
            "I recommend doing the conversion explicitly with one of these functions:",
        ));
        stack([
            fill_sep(first),
            vcat([
                green(text("round")).append(text(" 3.5     == 4")),
                green(text("floor")).append(text(" 3.5     == 3")),
                green(text("ceiling")).append(text(" 3.5   == 4")),
                green(text("truncate")).append(text(" 3.5  == 3")),
            ]),
            link(
                "Note",
                "Read",
                "implicit-casts",
                "to learn why Elm does not implicitly convert Ints to Floats.",
            ),
        ])
    } else {
        let mut seeing = must_be(direction, "//", "an", "Int");
        seeing.extend(words("but instead I am seeing:"));
        lone_type(localizer, ty, expected, fill_sep(seeing), Vec::new())
    };
    (
        reflow("The (//) operator is specifically for integer division:"),
        details,
    )
}

/// `badBool`.
fn bad_bool(
    localizer: &Localizer,
    op: &str,
    direction: &str,
    ty: &t::Type,
    expected: &t::Type,
) -> (Doc, Doc) {
    let mut seeing = words("Both sides of");
    seeing.push(text(format!("({op})")));
    seeing.extend(words("must be"));
    seeing.push(yellow(text("Bool")));
    seeing.extend(words("values, but the"));
    seeing.push(text(direction.to_owned()));
    seeing.extend(words("side is:"));
    (
        reflow("I am struggling with this boolean operation:"),
        lone_type(localizer, ty, expected, fill_sep(seeing), Vec::new()),
    )
}

/// `badCompLeft`.
fn bad_comp_left(
    localizer: &Localizer,
    category: &Category,
    op: &str,
    direction: &str,
    ty: &t::Type,
    expected: &t::Type,
) -> (Doc, Doc) {
    let mut works = vec![
        text("But"),
        text(format!("({op})")),
        text("only"),
        text("works"),
        text("on"),
    ];
    works.extend([
        yellow(text("Int")).append(text(",")),
        yellow(text("Float")).append(text(",")),
        yellow(text("Char")).append(text(",")),
        text("and"),
        yellow(text("String")),
    ]);
    works.extend(words(
        "values. It can work on lists and tuples of comparable values as well, but it is usually better to find a different path.",
    ));
    (
        reflow("I cannot do a comparison with this value:"),
        lone_type(
            localizer,
            ty,
            expected,
            reflow(&add_category(
                &format!("The {direction} side of ({op}) is"),
                category,
            )),
            vec![fill_sep(works)],
        ),
    )
}

/// `badCompRight`.
fn bad_comp_right(localizer: &Localizer, op: &str, ty: &t::Type, expected: &t::Type) -> RightDocs {
    RightDocs::Both((
        reflow(&format!("I need both sides of ({op}) to be the same type:")),
        type_comparison(
            localizer,
            expected,
            ty,
            &format!("The left side of ({op}) is:"),
            "But the right side is:",
            vec![reflow(&format!(
                "I cannot compare different types though! Which side of ({op}) is the problem?"
            ))],
        ),
    ))
}

/// `badEquality`.
fn bad_equality(localizer: &Localizer, op: &str, ty: &t::Type, expected: &t::Type) -> RightDocs {
    let hint = if is_float(ty) || is_float(expected) {
        simple_note(
            "Equality on floats is not 100% reliable due to the design of IEEE 754. I recommend a check like (abs (x - y) < 0.0001) instead.",
        )
    } else {
        reflow("Different types can never be equal though! Which side is messed up?")
    };
    RightDocs::Both((
        reflow(&format!("I need both sides of ({op}) to be the same type:")),
        type_comparison(
            localizer,
            expected,
            ty,
            &format!("The left side of ({op}) is:"),
            "But the right side is:",
            vec![hint],
        ),
    ))
}
