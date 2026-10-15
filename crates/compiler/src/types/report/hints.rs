//! The hints a comparison's first problem gives (`problemToHint`).

use nomi_base::diagnostic::doc::{
    Conjunction, Doc, comma_sep, dull_yellow as yellow, green, hint, labelled, link, reflow_link,
    simple_hint, text, words,
};
use nomi_base::suggest::nearest;

use super::t::{Direction, Problem, Type};
use crate::types::{SuperType, is_string};

/// `problemsToHint`: the first problem's.
pub(super) fn problems_to_hint(problems: &[Problem]) -> Vec<Doc> {
    problems.first().map(problem_to_hint).unwrap_or_default()
}

/// `problemToHint`.
#[expect(
    clippy::too_many_lines,
    reason = "upstream's `problemToHint`, one hint per problem"
)]
fn problem_to_hint(problem: &Problem) -> Vec<Doc> {
    // A hint whose one green word is `function`.
    let fancy = |before: &str, function: &str, after: &str| {
        vec![hint(
            words(before).chain([green(function)]).chain(words(after)),
        )]
    };
    match problem {
        Problem::IntFloat => {
            let docs = (words("Read"))
                .chain([Doc::link("implicit-casts")])
                .chain(words(
                    "to learn why Elm does not implicitly convert Ints to Floats. Use",
                ))
                .chain([green("toFloat"), text("and"), green("round")])
                .chain(words("to do explicit conversions."));
            vec![labelled("Note", docs)]
        }
        Problem::StringFromInt => fancy(
            "Want to convert an Int into a String? Use the",
            "String.fromInt",
            "function!",
        ),
        Problem::StringFromFloat => fancy(
            "Want to convert a Float into a String? Use the",
            "String.fromFloat",
            "function!",
        ),
        Problem::StringToInt => fancy(
            "Want to convert a String into an Int? Use the",
            "String.toInt",
            "function!",
        ),
        Problem::StringToFloat => fancy(
            "Want to convert a String into a Float? Use the",
            "String.toFloat",
            "function!",
        ),
        Problem::AnythingToBool => vec![simple_hint(
            "Elm does not have “truthiness” such that ints and strings and lists are automatically converted to booleans. Do that conversion explicitly!",
        )],
        Problem::AnythingFromMaybe => fancy(
            "Use",
            "Maybe.withDefault",
            "to handle possible errors. Longer term, it is usually better to write out the full `case` though!",
        ),
        Problem::ArityMismatch { actual, expected } => vec![simple_hint(&if actual < expected {
            format!(
                "It looks like it takes too few arguments. I was expecting {} more.",
                expected - actual
            )
        } else {
            format!(
                "It looks like it takes too many arguments. I see {} extra.",
                actual - expected
            )
        })],
        Problem::BadFlexSuper {
            direction,
            super_type: s,
            other,
        } => match other {
            Type::Infinite | Type::Error | Type::FlexVar(_) => Vec::new(),
            Type::FlexSuper(s2, _) => bad_flex_flex_super(*s, *s2),
            Type::RigidVar(y) => bad_rigid_var(y, to_a_super_thing(*s)),
            Type::RigidSuper(s2, _) => bad_rigid_super(*s2, to_a_super_thing(*s)),
            Type::Lambda(..)
            | Type::Type { .. }
            | Type::Record(..)
            | Type::Unit
            | Type::Tuple(..)
            | Type::Alias { .. } => bad_flex_super(*direction, *s, other),
        },
        Problem::BadRigidVar { name: x, other: ty } => match ty {
            Type::Lambda(..) => bad_rigid_var(x, "a function"),
            Type::Infinite | Type::Error | Type::FlexVar(_) => Vec::new(),
            Type::FlexSuper(s, _) => bad_rigid_var(x, to_a_super_thing(*s)),
            Type::RigidVar(y) | Type::RigidSuper(_, y) => bad_double_rigid(x, y),
            Type::Type { name: n, .. } | Type::Alias { name: n, .. } => {
                bad_rigid_var(x, &format!("a `{n}` value"))
            }
            Type::Record(..) => bad_rigid_var(x, "a record"),
            Type::Unit => bad_rigid_var(x, "a unit value"),
            Type::Tuple(..) => bad_rigid_var(x, "a tuple"),
        },
        Problem::BadRigidSuper {
            super_type: s,
            name: x,
            other: ty,
        } => match ty {
            Type::Lambda(..) => bad_rigid_super(*s, "a function"),
            Type::Infinite | Type::Error | Type::FlexVar(_) => Vec::new(),
            Type::FlexSuper(s2, _) => bad_rigid_super(*s, to_a_super_thing(*s2)),
            Type::RigidVar(y) | Type::RigidSuper(_, y) => bad_double_rigid(x, y),
            Type::Type { name: n, .. } | Type::Alias { name: n, .. } => {
                bad_rigid_super(*s, &format!("a `{n}` value"))
            }
            Type::Record(..) => bad_rigid_super(*s, "a record"),
            Type::Unit => bad_rigid_super(*s, "a unit value"),
            Type::Tuple(..) => bad_rigid_super(*s, "a tuple"),
        },
        Problem::FieldsMissing(fields) => {
            let docs: Vec<Doc> = fields.iter().map(|f| green(f.as_str())).collect();
            match &docs[..] {
                [] => Vec::new(),
                [f1] => {
                    let docs = words("Looks like the").chain([f1.clone()]);
                    vec![hint(docs.chain(words("field is missing.")))]
                }
                _ => {
                    let docs = words("Looks like fields").chain(comma_sep(Conjunction::And, docs));
                    vec![hint(docs.chain(words("are missing.")))]
                }
            }
        }
        Problem::FieldTypo {
            typo,
            fields: possibilities,
        } => match nearest(typo, possibilities.clone(), 1).first() {
            None => Vec::new(),
            Some(nearest) => {
                let docs = (words("Seems like a record field typo. Maybe"))
                    .chain([yellow(typo.as_str())])
                    .chain(words("should be"))
                    .chain([green(nearest.as_str()).append(text("?"))]);
                vec![
                    hint(docs),
                    simple_hint(
                        "Can more type annotations be added? Type annotations always help me give more specific messages, and I think they could help a lot in this case!",
                    ),
                ]
            }
        },
    }
}

/// The link to advice on type annotations.
fn more_advice() -> Doc {
    reflow_link("Read", "type-annotations", "for more advice!")
}

/// `badRigidVar`.
fn bad_rigid_var(name: &str, a_thing: &str) -> Vec<Doc> {
    vec![
        simple_hint(&format!(
            "Your type annotation uses type variable `{name}` which means ANY type of value can flow through, but your code is saying it specifically wants {a_thing}. Maybe change your type annotation to be more specific? Maybe change the code to be more general?"
        )),
        more_advice(),
    ]
}

/// `badDoubleRigid`.
fn bad_double_rigid(x: &str, y: &str) -> Vec<Doc> {
    vec![
        simple_hint(&format!(
            "Your type annotation uses `{x}` and `{y}` as separate type variables. Your code seems to be saying they are the same though. Maybe they should be the same in your type annotation? Maybe your code uses them in a weird way?"
        )),
        more_advice(),
    ]
}

/// `toASuperThing`.
fn to_a_super_thing(s: SuperType) -> &'static str {
    match s {
        SuperType::Number => "a `number` value",
        SuperType::Comparable => "a `comparable` value",
        SuperType::CompAppend => "a `compappend` value",
        SuperType::Appendable => "an `appendable` value",
    }
}

/// `badFlexSuper`.
fn bad_flex_super(direction: Direction, s: SuperType, ty: &Type) -> Vec<Doc> {
    match s {
        SuperType::Comparable => match ty {
            Type::Record(..) => vec![link(
                "Hint",
                "I do not know how to compare records. I can only compare ints, floats, chars, strings, lists of comparable values, and tuples of comparable values. Check out",
                "comparing-records",
                "for ideas on how to proceed.",
            )],
            Type::Type { name, .. } => vec![
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
        SuperType::Appendable => vec![simple_hint("I only know how to append strings and lists.")],
        SuperType::CompAppend => vec![simple_hint(
            "Only strings and lists are both comparable and appendable.",
        )],
        SuperType::Number => match ty {
            Type::Type { home, name, .. } if is_string(home, name) => {
                let (function, into) = match direction {
                    Direction::Have => ("String.fromInt", "to convert it to a string?"),
                    Direction::Need => ("String.toInt", "to convert it to an integer?"),
                };
                vec![hint(
                    words("Try using")
                        .chain([green(function)])
                        .chain(words(into)),
                )]
            }
            _ => vec![hint(
                [text("Only"), green("Int"), text("and"), green("Float")]
                    .into_iter()
                    .chain(words("values work as numbers.")),
            )],
        },
    }
}

/// `badRigidSuper`.
fn bad_rigid_super(s: SuperType, a_thing: &str) -> Vec<Doc> {
    let (super_type, many_things) = match s {
        SuperType::Number => ("number", "ints AND floats"),
        SuperType::Comparable => (
            "comparable",
            "ints, floats, chars, strings, lists, and tuples",
        ),
        SuperType::Appendable => ("appendable", "strings AND lists"),
        SuperType::CompAppend => ("compappend", "strings AND lists"),
    };
    vec![
        simple_hint(&format!(
            "The `{super_type}` in your type annotation is saying that {many_things} can flow through, but your code is saying it specifically wants {a_thing}. Maybe change your type annotation to be more specific? Maybe change the code to be more general?"
        )),
        more_advice(),
    ]
}

/// `badFlexFlexSuper`.
fn bad_flex_flex_super(s1: SuperType, s2: SuperType) -> Vec<Doc> {
    let like_this = |s| match s {
        SuperType::Number => "a number",
        SuperType::Comparable => "comparable",
        SuperType::CompAppend => "a compappend",
        SuperType::Appendable => "appendable",
    };
    vec![simple_hint(&format!(
        "There are no values in Elm that are both {} and {}.",
        like_this(s1),
        like_this(s2)
    ))]
}
