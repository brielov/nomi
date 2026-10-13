//! Calls and operators: each operator's own explanation of a bad side.

use super::*;

/// `countArgs`.
pub(super) fn count_args(ty: &t::Type) -> usize {
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
pub(super) fn op_left(
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
pub(super) enum RightDocs {
    Both((Doc, Doc)),
    Right((Doc, Doc)),
}

/// `opRightToDocs`.
pub(super) fn op_right(
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
    if let AppendType::Number(thing, from) = append_type(ty) {
        let mut first = words("The (++) operator can append List and String values, but not");
        first.push(yellow(text(thing)));
        first.extend(words("values like this:"));
        let mut second = words("Try using");
        second.push(green(text(from)));
        second.extend(words(
            "to turn it into a string? Or put it in [] to make it a list? Or switch to the (::) operator?",
        ));
        (fill_sep(first), fill_sep(second))
    } else {
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
#[expect(
    clippy::too_many_arguments,
    reason = "upstream's `badMath` takes these"
)]
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
