//! Calls and operators: each operator's own explanation of a bad side.

use nomi_base::diagnostic::doc::{
    Color, Doc, dull_yellow as yellow, fill_sep, green, hint, link, reflow, simple_hint,
    simple_note, stack, text, vcat, words,
};

use super::t::Type;
use super::{HintOrder, add_category, int_and_float_values, lone_type, type_comparison};
use crate::canonical::Home;
use crate::types::expected::Category;
use crate::types::localizer::Localizer;
use crate::types::{self, SuperType};

/// `countArgs`.
pub(super) fn count_args(ty: &Type) -> usize {
    match ty {
        Type::Lambda(_, _, rest) => 1 + rest.len(),
        Type::Infinite
        | Type::Error
        | Type::FlexVar(_)
        | Type::FlexSuper(..)
        | Type::RigidVar(_)
        | Type::RigidSuper(..)
        | Type::Type(..)
        | Type::Record(..)
        | Type::Unit
        | Type::Tuple(..)
        | Type::Alias(..) => 0,
    }
}

fn atom(ty: &Type, test: fn(&Home, &str) -> bool) -> bool {
    matches!(ty, Type::Type(home, name, args) if args.is_empty() && test(home, name))
}

fn is_int(ty: &Type) -> bool {
    atom(ty, types::is_int)
}

fn is_float(ty: &Type) -> bool {
    atom(ty, types::is_float)
}

fn is_string(ty: &Type) -> bool {
    atom(ty, types::is_string)
}

/// The element type of a list type.
fn list_element(ty: &Type) -> Option<&Type> {
    match ty {
        Type::Type(home, name, args) if types::is_list(home, name) => match args.as_slice() {
            [element] => Some(element),
            _ => None,
        },
        _ => None,
    }
}

/// What a report says of a bad operand: the problem, and the details.
pub(super) struct Explanation {
    pub(super) problem: Doc,
    pub(super) details: Doc,
}

impl Explanation {
    fn new(problem: Doc, details: Doc) -> Explanation {
        Explanation { problem, details }
    }
}

/// Which operand of an operator a report is about.
#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
}

impl Side {
    fn as_str(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }
}

/// The arithmetic operators `badMath` explains.
#[derive(Clone, Copy)]
enum MathOp {
    Add,
    Mul,
    Sub,
    Pow,
}

impl MathOp {
    fn symbol(self) -> &'static str {
        match self {
            MathOp::Add => "+",
            MathOp::Mul => "*",
            MathOp::Sub => "-",
            MathOp::Pow => "^",
        }
    }

    fn operation(self) -> &'static str {
        match self {
            MathOp::Add => "Addition",
            MathOp::Mul => "Multiplication",
            MathOp::Sub => "Subtraction",
            MathOp::Pow => "Exponentiation",
        }
    }
}

/// The explanations `opLeftToDocs` and `opRightToDocs` share: of an
/// arithmetic or boolean operator's bad `side`.
fn arithmetic(
    localizer: &Localizer,
    category: &Category,
    op: &str,
    side: Side,
    ty: &Type,
    expected: &Type,
) -> Option<Explanation> {
    let math = |math_op| bad_math(localizer, category, math_op, side, ty, expected, Vec::new());
    Some(match op {
        "+" if is_string(ty) => bad_string_add(),
        "+" if list_element(ty).is_some() => bad_list_add(localizer, category, side, ty, expected),
        "+" => math(MathOp::Add),
        "*" if list_element(ty).is_some() => bad_list_mul(localizer, category, side, ty, expected),
        "*" => math(MathOp::Mul),
        "-" => math(MathOp::Sub),
        "^" => math(MathOp::Pow),
        "/" => bad_f_div(localizer, side, ty, expected),
        "//" => bad_i_div(localizer, side, ty, expected),
        "&&" | "||" => bad_bool(localizer, op, side, ty, expected),
        _ => return None,
    })
}

/// `opLeftToDocs`.
pub(super) fn op_left(
    localizer: &Localizer,
    category: &Category,
    op: &str,
    ty: &Type,
    expected: &Type,
) -> Explanation {
    if let Some(explanation) = arithmetic(localizer, category, op, Side::Left, ty, expected) {
        return explanation;
    }
    match op {
        "<" | ">" | "<=" | ">=" => bad_comp_left(localizer, category, op, ty, expected),
        "++" => bad_append_left(localizer, category, ty, expected),
        "<|" => Explanation::new(
            reflow("The left side of (<|) needs to be a function so I can pipe arguments to it!"),
            lone_type(
                localizer,
                ty,
                expected,
                reflow(&add_category("I am seeing", category)),
                vec![reflow("This needs to be some kind of function though!")],
            ),
        ),
        _ => Explanation::new(
            reflow(&format!("The left argument of ({op}) is causing problems:")),
            type_comparison(
                localizer,
                ty,
                expected,
                &add_category("The left argument is", category),
                &format!("But ({op}) needs the left argument to be:"),
                Vec::new(),
                HintOrder::ContextFirst,
            ),
        ),
    }
}

/// `RightDocs`: whether a report highlights both operands or the right.
pub(super) enum RightDocs {
    Both(Explanation),
    Right(Explanation),
}

/// Which way Int and Float are mixed: `FloatInt` with the Float on the
/// left.
#[derive(Clone, Copy)]
enum Cast {
    FloatInt,
    IntFloat,
}

/// `opRightToDocs`.
pub(super) fn op_right(
    localizer: &Localizer,
    category: &Category,
    op: &str,
    ty: &Type,
    expected: &Type,
) -> RightDocs {
    let cast = if is_float(expected) && is_int(ty) {
        Some(Cast::FloatInt)
    } else if is_int(expected) && is_float(ty) {
        Some(Cast::IntFloat)
    } else {
        None
    };
    if let ("+" | "*" | "-" | "^", Some(cast)) = (op, cast) {
        return bad_cast(op, cast);
    }
    if let Some(explanation) = arithmetic(localizer, category, op, Side::Right, ty, expected) {
        return RightDocs::Right(explanation);
    }
    match op {
        "<" | ">" | "<=" | ">=" => bad_comp_right(localizer, op, ty, expected),
        "==" | "/=" => bad_equality(localizer, op, ty, expected),
        "::" => bad_cons_right(localizer, category, ty, expected),
        "++" => bad_append_right(localizer, category, ty, expected),
        "<|" => RightDocs::Right(Explanation::new(
            reflow("I cannot send this through the (<|) pipe:"),
            type_comparison(
                localizer,
                ty,
                expected,
                "The argument is:",
                "But (<|) is piping it to a function that expects:",
                Vec::new(),
                HintOrder::ContextFirst,
            ),
        )),
        "|>" => match (ty, expected) {
            (Type::Lambda(expected_arg, _, _), Type::Lambda(arg, _, _)) => {
                RightDocs::Right(Explanation::new(
                    reflow("This function cannot handle the argument sent through the (|>) pipe:"),
                    type_comparison(
                        localizer,
                        arg,
                        expected_arg,
                        "The argument is:",
                        "But (|>) is piping it to a function that expects:",
                        Vec::new(),
                        HintOrder::ContextFirst,
                    ),
                ))
            }
            _ => RightDocs::Right(Explanation::new(
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
    ty: &Type,
    expected: &Type,
) -> RightDocs {
    RightDocs::Right(Explanation::new(
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
            HintOrder::ContextFirst,
        ),
    ))
}

/// `badConsRight`.
fn bad_cons_right(
    localizer: &Localizer,
    category: &Category,
    ty: &Type,
    expected: &Type,
) -> RightDocs {
    match (list_element(ty), list_element(expected)) {
        (Some(actual_element), Some(expected_element)) => {
            let hint = if list_element(expected_element).is_some() {
                simple_hint(
                    "Are you trying to append two lists? The (++) operator appends lists, whereas the (::) operator is only for adding ONE element to a list.",
                )
            } else {
                reflow("Lists need ALL elements to be the same type though.")
            };
            RightDocs::Both(Explanation::new(
                reflow("I am having trouble with this (::) operator:"),
                type_comparison(
                    localizer,
                    expected_element,
                    actual_element,
                    "The left side of (::) is:",
                    "But you are trying to put that into a list filled with:",
                    vec![hint],
                    HintOrder::ContextFirst,
                ),
            ))
        }
        (Some(_), None) => bad_op_right_fallback(localizer, category, "::", ty, expected),
        (None, _) => {
            let details = (words("But (::) needs a"))
                .chain([yellow("List")])
                .chain(words("on the right."));
            RightDocs::Right(Explanation::new(
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
    /// A number, called `thing`, which `convert` makes a string.
    Number {
        thing: &'static str,
        convert: &'static str,
    },
    String,
    List,
    Other,
}

/// `toAppendType`.
fn append_type(ty: &Type) -> AppendType {
    let number = |thing, convert| AppendType::Number { thing, convert };
    match ty {
        Type::Type(home, name, _) if types::is_int(home, name) => number("Int", "String.fromInt"),
        Type::Type(home, name, _) if types::is_float(home, name) => {
            number("Float", "String.fromFloat")
        }
        Type::Type(home, name, _) if types::is_string(home, name) => AppendType::String,
        Type::Type(home, name, _) if types::is_list(home, name) => AppendType::List,
        Type::FlexSuper(SuperType::Number, _) => number("number", "String.fromInt"),
        Type::Type(..)
        | Type::Lambda(..)
        | Type::Infinite
        | Type::Error
        | Type::FlexVar(_)
        | Type::FlexSuper(..)
        | Type::RigidVar(_)
        | Type::RigidSuper(..)
        | Type::Record(..)
        | Type::Unit
        | Type::Tuple(..)
        | Type::Alias(..) => AppendType::Other,
    }
}

/// `badAppendLeft`.
fn bad_append_left(
    localizer: &Localizer,
    category: &Category,
    ty: &Type,
    expected: &Type,
) -> Explanation {
    if let AppendType::Number { thing, convert } = append_type(ty) {
        let first = words("The (++) operator can append List and String values, but not")
            .chain([yellow(thing)])
            .chain(words("values like this:"));
        let second = (words("Try using")).chain([green(convert)]).chain(words(
            "to turn it into a string? Or put it in [] to make it a list? Or switch to the (::) operator?",
        ));
        Explanation::new(fill_sep(first), fill_sep(second))
    } else {
        let details = (words("But the (++) operator is only for appending"))
            .chain([
                yellow("List"),
                text("and"),
                yellow("String"),
                text("values."),
            ])
            .chain(words("Maybe put this value in [] to make it a list?"));
        Explanation::new(
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
    ty: &Type,
    expected: &Type,
) -> RightDocs {
    let appending = |kind, thing| {
        let first = (words("I thought I was appending"))
            .chain([yellow(kind)])
            .chain(words("values here, not"))
            .chain([yellow(thing)])
            .chain(words("values like this:"));
        fill_sep(first)
    };
    match (append_type(expected), append_type(ty)) {
        (AppendType::String, AppendType::Number { thing, convert }) => {
            let second = (words("Try using"))
                .chain([green(convert)])
                .chain(words("to turn it into a string?"));
            RightDocs::Right(Explanation::new(
                appending("String", thing),
                fill_sep(second),
            ))
        }
        (AppendType::List, AppendType::Number { thing, .. }) => RightDocs::Right(Explanation::new(
            appending("List", thing),
            reflow("Try putting it in [] to make it a list?"),
        )),
        (AppendType::String, AppendType::List) => same_sides("String", "List"),
        (AppendType::List, AppendType::String) => same_sides("List", "String"),
        _ => RightDocs::Both(Explanation::new(
            reflow("The (++) operator cannot append these two values:"),
            type_comparison(
                localizer,
                expected,
                ty,
                "I already figured out that the left side of (++) is:",
                &add_category("But this clashes with the right side, which is", category),
                Vec::new(),
                HintOrder::ContextFirst,
            ),
        )),
    }
}

/// A String on one side of (++) and a List on the other.
fn same_sides(left: &str, right: &str) -> RightDocs {
    let details = (words("I see a"))
        .chain([yellow(left)])
        .chain(words("on the left and a"))
        .chain([yellow(right)])
        .chain(words(
            "on the right. Which should it be? Does the string need [] around it to become a list?",
        ));
    RightDocs::Both(Explanation::new(
        reflow("The (++) operator needs the same type of value on both sides:"),
        fill_sep(details),
    ))
}

/// The note that Elm doesn't convert Ints to Floats implicitly.
fn implicit_casts() -> Doc {
    link(
        "Note",
        "Read",
        "implicit-casts",
        "to learn why Elm does not implicitly convert Ints to Floats.",
    )
}

/// `badCast`: Int and Float on either side.
fn bad_cast(op: &str, cast: Cast) -> RightDocs {
    let an_int = [text("an"), yellow("Int")];
    let a_float = [text("a"), yellow("Float")];
    let (to_float, round) = (green("toFloat"), green("round"));
    let details = match cast {
        Cast::FloatInt => cast_help(a_float, an_int, round, to_float),
        Cast::IntFloat => cast_help(an_int, a_float, to_float, round),
    };
    RightDocs::Both(Explanation::new(
        reflow(&format!(
            "I need both sides of ({op}) to be the exact same type. Both Int or both Float."
        )),
        details,
    ))
}

/// `badCastHelp`.
fn cast_help(left: [Doc; 2], right: [Doc; 2], fix_left: Doc, fix_right: Doc) -> Doc {
    let first = (words("But I see"))
        .chain(left)
        .chain(words("on the left and"))
        .chain(right)
        .chain(words("on the right."));
    let second = [text("Use"), fix_left]
        .into_iter()
        .chain(words("on the left (or"))
        .chain([fix_right])
        .chain(words("on the right) to make both sides match!"));
    stack([fill_sep(first), fill_sep(second), implicit_casts()])
}

/// `badStringAdd`.
fn bad_string_add() -> Explanation {
    let first = (words("I cannot do addition with"))
        .chain([yellow("String")])
        .chain(words("values like this one:"));
    let works = words("The (+) operator only works with").chain(int_and_float_values());
    let hint = hint(
        (words("Switch to the"))
            .chain([green("(++)")])
            .chain(words("operator to append strings!")),
    );
    Explanation::new(fill_sep(first), stack([fill_sep(works), hint]))
}

/// `badListAdd`.
fn bad_list_add(
    localizer: &Localizer,
    category: &Category,
    side: Side,
    ty: &Type,
    expected: &Type,
) -> Explanation {
    let works = words("But (+) only works with").chain(int_and_float_values());
    let hint = hint(
        (words("Switch to the"))
            .chain([green("(++)")])
            .chain(words("operator to append lists!")),
    );
    let side = side.as_str();
    Explanation::new(
        text("I cannot do addition with lists:"),
        lone_type(
            localizer,
            ty,
            expected,
            reflow(&add_category(
                &format!("The {side} side of (+) is"),
                category,
            )),
            vec![fill_sep(works), hint],
        ),
    )
}

/// `badListMul`.
fn bad_list_mul(
    localizer: &Localizer,
    category: &Category,
    side: Side,
    ty: &Type,
    expected: &Type,
) -> Explanation {
    let hint = hint(
        (words("Maybe you want"))
            .chain([green("List.repeat")])
            .chain(words("to build a list of repeated values?")),
    );
    bad_math(
        localizer,
        category,
        MathOp::Mul,
        side,
        ty,
        expected,
        vec![hint],
    )
}

/// `badMath`.
fn bad_math(
    localizer: &Localizer,
    category: &Category,
    op: MathOp,
    side: Side,
    ty: &Type,
    expected: &Type,
    other_hints: Vec<Doc>,
) -> Explanation {
    let (symbol, side) = (op.symbol(), side.as_str());
    let but = format!("But ({symbol}) only works with");
    let works = words(&but).chain(int_and_float_values());
    let mut further = vec![fill_sep(works)];
    further.extend(other_hints);
    Explanation::new(
        reflow(&format!(
            "{} does not work with this value:",
            op.operation()
        )),
        lone_type(
            localizer,
            ty,
            expected,
            reflow(&add_category(
                &format!("The {side} side of ({symbol}) is"),
                category,
            )),
            further,
        ),
    )
}

/// The opening of `badFDiv` and `badIDiv`: `The left side of (op) must
/// be a Type,`.
fn must_be(side: Side, op: &str, article: &str, ty: &str) -> Vec<Doc> {
    let side = side.as_str();
    let mut docs: Vec<Doc> =
        words(&format!("The {side} side of ({op}) must be {article}")).collect();
    docs.push(yellow(ty).append(text(",")));
    docs
}

/// `badFDiv`.
fn bad_f_div(localizer: &Localizer, side: Side, ty: &Type, expected: &Type) -> Explanation {
    let details = if is_int(ty) {
        let first = (must_be(side, "/", "a", "Float").into_iter())
            .chain(words("but I am seeing an"))
            .chain([yellow("Int").append(text("."))])
            .chain(words("I recommend:"));
        stack([
            fill_sep(first),
            vcat([
                green("toFloat")
                    .append(text(" for explicit conversions     "))
                    .append(text("(toFloat 5 / 2) == 2.5").color(Color::Black)),
                green("(//)   ")
                    .append(text(" for integer division         "))
                    .append(text("(5 // 2)        == 2").color(Color::Black)),
            ]),
            implicit_casts(),
        ])
    } else {
        let seeing = must_be(side, "/", "a", "Float")
            .into_iter()
            .chain(words("but instead I am seeing:"));
        lone_type(localizer, ty, expected, fill_sep(seeing), Vec::new())
    };
    Explanation::new(
        reflow("The (/) operator is specifically for floating-point division:"),
        details,
    )
}

/// `badIDiv`.
fn bad_i_div(localizer: &Localizer, side: Side, ty: &Type, expected: &Type) -> Explanation {
    let details = if is_float(ty) {
        let first = (must_be(side, "//", "an", "Int").into_iter())
            .chain(words("but I am seeing a"))
            .chain([yellow("Float").append(text("."))])
            .chain(words(
                "I recommend doing the conversion explicitly with one of these functions:",
            ));
        stack([
            fill_sep(first),
            vcat([
                green("round").append(text(" 3.5     == 4")),
                green("floor").append(text(" 3.5     == 3")),
                green("ceiling").append(text(" 3.5   == 4")),
                green("truncate").append(text(" 3.5  == 3")),
            ]),
            implicit_casts(),
        ])
    } else {
        let seeing = must_be(side, "//", "an", "Int")
            .into_iter()
            .chain(words("but instead I am seeing:"));
        lone_type(localizer, ty, expected, fill_sep(seeing), Vec::new())
    };
    Explanation::new(
        reflow("The (//) operator is specifically for integer division:"),
        details,
    )
}

/// `badBool`.
fn bad_bool(
    localizer: &Localizer,
    op: &str,
    side: Side,
    ty: &Type,
    expected: &Type,
) -> Explanation {
    let seeing = (words("Both sides of"))
        .chain([text(format!("({op})"))])
        .chain(words("must be"))
        .chain([yellow("Bool")])
        .chain(words("values, but the"))
        .chain([text(side.as_str())])
        .chain(words("side is:"));
    Explanation::new(
        reflow("I am struggling with this boolean operation:"),
        lone_type(localizer, ty, expected, fill_sep(seeing), Vec::new()),
    )
}

/// `badCompLeft`.
fn bad_comp_left(
    localizer: &Localizer,
    category: &Category,
    op: &str,
    ty: &Type,
    expected: &Type,
) -> Explanation {
    let but = format!("But ({op}) only works on");
    let works = (words(&but))
        .chain([
            yellow("Int").append(text(",")),
            yellow("Float").append(text(",")),
            yellow("Char").append(text(",")),
            text("and"),
            yellow("String"),
        ])
        .chain(words(
            "values. It can work on lists and tuples of comparable values as well, but it is usually better to find a different path.",
        ));
    Explanation::new(
        reflow("I cannot do a comparison with this value:"),
        lone_type(
            localizer,
            ty,
            expected,
            reflow(&add_category(
                &format!("The left side of ({op}) is"),
                category,
            )),
            vec![fill_sep(works)],
        ),
    )
}

/// `badCompRight`.
fn bad_comp_right(localizer: &Localizer, op: &str, ty: &Type, expected: &Type) -> RightDocs {
    RightDocs::Both(Explanation::new(
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
            HintOrder::ContextFirst,
        ),
    ))
}

/// `badEquality`.
fn bad_equality(localizer: &Localizer, op: &str, ty: &Type, expected: &Type) -> RightDocs {
    let hint = if is_float(ty) || is_float(expected) {
        simple_note(
            "Equality on floats is not 100% reliable due to the design of IEEE 754. I recommend a check like (abs (x - y) < 0.0001) instead.",
        )
    } else {
        reflow("Different types can never be equal though! Which side is messed up?")
    };
    RightDocs::Both(Explanation::new(
        reflow(&format!("I need both sides of ({op}) to be the same type:")),
        type_comparison(
            localizer,
            expected,
            ty,
            &format!("The left side of ({op}) is:"),
            "But the right side is:",
            vec![hint],
            HintOrder::ContextFirst,
        ),
    ))
}
