//! The hints a comparison's first problem gives (`problemToHint`).

use super::*;

/// `problemsToHint`: the first problem's.
pub(super) fn problems_to_hint(problems: &[TProblem]) -> Vec<Doc> {
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
