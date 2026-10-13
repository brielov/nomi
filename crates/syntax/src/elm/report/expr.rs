//! The reports on expressions: `toExprReport` with the context it
//! carries, `toOperatorReport` and `toFuncReport`. Blocks, collections
//! and patterns have their own modules.

use std::borrow::Cow;

use nomi_base::Compat;
use nomi_base::diagnostic::doc::{self, dull_yellow, green, reflow, stack, vcat};
use nomi_base::diagnostic::{Doc, Problem};

use super::blocks::{note_for_case_error, note_for_case_indent_error};
use super::pattern::PContext;
use super::{Reporter, to_region};
use crate::elm::error::{BadOperator, Expr, Func, Position};

/// `Context`: what an expression is in, which some reports name.
#[derive(Clone, Copy)]
pub(super) enum Context<'a> {
    /// `InNode`: inside `node`, which starts at `start`.
    Node {
        node: Node,
        start: Position,
        parent: &'a Context<'a>,
    },
    /// `InDef`.
    Def(&'a str, Position),
    /// `InDestruct`.
    Destruct(Position),
}

/// `Node`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Node {
    Record,
    Parens,
    List,
    Func,
    Cond,
    Then,
    Else,
    Case,
    Branch,
}

impl<'a> Context<'a> {
    /// `InNode`: the context of an expression in `node`, which starts at
    /// `start`, in this one.
    pub(super) fn node(&'a self, node: Node, start: Position) -> Context<'a> {
        Context::Node {
            node,
            start,
            parent: self,
        }
    }

    /// `getDefName`.
    fn def_name(&self) -> Option<&'a str> {
        match *self {
            Context::Destruct(_) => None,
            Context::Def(name, _) => Some(name),
            Context::Node { parent, .. } => parent.def_name(),
        }
    }

    /// `isWithin`, which looks at the innermost node only.
    fn is_within(&self, node: Node) -> bool {
        matches!(*self, Context::Node { node: actual, .. } if actual == node)
    }
}

impl Reporter<'_, '_> {
    /// `toExprReport`.
    pub(super) fn expr(&self, context: &Context<'_>, expr: &Expr, start: Position) -> Problem {
        match expr {
            Expr::Let(let_, cur) => self.let_(context, let_, *cur),
            Expr::Case(case, cur) => self.case(context, case, *cur),
            Expr::If(if_, cur) => self.if_(context, if_, *cur),
            Expr::List(list, cur) => self.list(context, list, *cur),
            Expr::Record(record, cur) => self.record(context, record, *cur),
            Expr::Tuple(tuple, cur) => self.tuple(context, tuple, *cur),
            Expr::Func(func, cur) => self.func(context, func, *cur),
            Expr::Dot(cur) => self.at(
                "EXPECTING RECORD ACCESSOR",
                to_region(*cur),
                reflow("I was expecting to see a record accessor here:"),
                accessor_example(),
            ),
            Expr::Access(cur) => self.at(
                "EXPECTING RECORD ACCESSOR",
                to_region(*cur),
                reflow("I am trying to parse a record accessor here:"),
                stack([
                    accessor_example(),
                    doc::simple_note("Record field names must start with a lower case letter!"),
                ]),
            ),
            Expr::OperatorRight(op, cur) => self.missing_operand(start, *cur, op),
            Expr::OperatorReserved(operator, cur) => self.operator(context, *operator, *cur),
            Expr::Start(cur) => self.missing_expression(context, *cur),
            Expr::Char(char, cur) => self.char(char, *cur),
            Expr::String(string, cur) => self.string(string, *cur),
            Expr::Number(number, cur) => self.number(number, *cur),
            Expr::ShaderEndless(cur) => self.shader_endless(*cur),
            Expr::ShaderNotUtf8(cur) => self.not_utf8(*cur),
            Expr::ShaderProblem(problem, cur) => self.shader_problem(problem, *cur),
            Expr::Space(space, cur) => self.space(*space, *cur),
            Expr::IndentOperatorRight(op, cur) => self.stuck(
                "MISSING EXPRESSION",
                start,
                *cur,
                reflow(&format!(
                    "I was expecting to see an expression after this {op} operator:"
                )),
                stack([
                    fill![
                        "You can just put anything for now, like",
                        dull_yellow("42"),
                        "or",
                        dull_yellow("\"hello\"").append(Doc::from(".")),
                        "Once there is something there, I can probably give a more specific hint!"
                    ],
                    doc::simple_note(&format!(
                        "I may be getting confused by your indentation? The easiest way to make \
                         sure this is not an indentation problem is to put the expression on the \
                         right of the {op} operator on the same line."
                    )),
                ]),
            ),
        }
    }

    /// `OperatorRight`: no expression after the operator `op`.
    fn missing_operand(&self, start: Position, cur: Position, op: &str) -> Problem {
        let (what, expecting) = match op {
            "-" | "+" | "*" | "/" | "^" => (
                "sign",
                fill![
                    "I was expecting to see an expression next. Something like",
                    dull_yellow("42"),
                    "or",
                    dull_yellow("1000"),
                    "that makes sense with a",
                    Doc::from(op),
                    "sign."
                ],
            ),
            "&&" | "||" => (
                "operator",
                fill![
                    "I was expecting to see an expression next. Something like",
                    dull_yellow("True"),
                    "or",
                    dull_yellow("False"),
                    "that makes sense with boolean logic."
                ],
            ),
            "|>" => (
                "operator",
                reflow("I was expecting to see a function next."),
            ),
            "<|" => (
                "operator",
                reflow("I was expecting to see an argument next."),
            ),
            _ => (
                "operator",
                reflow("I was expecting to see an expression next."),
            ),
        };
        self.stuck(
            "MISSING EXPRESSION",
            start,
            cur,
            reflow(&format!(
                "I just saw a {op} {what}, so I am getting stuck here:"
            )),
            expecting,
        )
    }

    /// `Start`: no expression where `context` wants one.
    fn missing_expression(&self, context: &Context<'_>, cur: Position) -> Problem {
        let (start, a_thing): (Position, Cow<'_, str>) = match *context {
            Context::Destruct(start) => (start, "a definition".into()),
            Context::Def(name, start) => (start, format!("the `{name}` definition").into()),
            Context::Node { node, start, .. } => (start, node.a_thing().into()),
        };
        self.stuck(
            "MISSING EXPRESSION",
            start,
            cur,
            reflow(&format!(
                "I am partway through parsing {a_thing}, but I got stuck here:"
            )),
            stack([
                fill![
                    "I was expecting to see an expression like",
                    dull_yellow("42"),
                    "or",
                    dull_yellow("\"hello\"").append(Doc::from(".")),
                    "Once there is something there, I can probably give a more specific hint!"
                ],
                doc::simple_note(&format!(
                    "This can also happen if {i}run into reserved words like `let` or `as` \
                     unexpectedly. Or if I run into operators in unexpected spots. Point is, \
                     there are a couple ways I can get confused and give sort of weird advice!",
                    // 0.19.1 drops the first "I"; 0.19.2 fixes it.
                    i = if self.compat == Compat::V0_19_1 {
                        ""
                    } else {
                        "I "
                    },
                )),
            ]),
        )
    }

    /// `toOperatorReport`.
    fn operator(&self, context: &Context<'_>, operator: BadOperator, cur: Position) -> Problem {
        let region = to_region(cur);
        match operator {
            BadOperator::BadDot => self.at(
                "UNEXPECTED SYMBOL",
                region,
                Doc::from("I was not expecting this dot:"),
                reflow(
                    "Dots are for record access and decimal points, so they cannot float around \
                     on their own. Maybe there is some extra whitespace?",
                ),
            ),
            BadOperator::BadPipe => self.at(
                "UNEXPECTED SYMBOL",
                region,
                reflow("I was not expecting this vertical bar:"),
                reflow(
                    "Vertical bars should only appear in custom type declarations. Maybe you \
                     want || instead?",
                ),
            ),
            BadOperator::BadArrow => self.bad_arrow(context, cur),
            BadOperator::BadEquals => self.bad_equals(context, cur),
            BadOperator::BadHasType => self.bad_has_type(context, cur),
        }
    }

    /// The `BadEquals` case of `toOperatorReport`.
    fn bad_equals(&self, context: &Context<'_>, cur: Position) -> Problem {
        self.at(
            "UNEXPECTED EQUALS",
            to_region(cur),
            reflow("I was not expecting to see this equals sign:"),
            stack([
                reflow("Maybe you want == instead? To check if two values are equal?"),
                doc::simple_note(&if context.is_within(Node::Record) {
                    "Records look like { x = 3, y = 4 } with the equals sign right after the \
                     field name. So maybe you forgot a comma?"
                        .to_owned()
                } else {
                    match context.def_name() {
                        None => "I may be getting confused by your indentation. I need all \
                                 definitions to be indented exactly the same amount, so if \
                                 this is meant to be a new definition, it may have too many \
                                 spaces in front of it."
                            .to_owned(),
                        Some(name) => format!(
                            "I may be getting confused by your indentation. I think I am \
                             still parsing the `{name}` definition. Is this supposed to be \
                             part of a definition after that? If so, the problem may be a \
                             bit before the equals sign. I need all definitions to be \
                             indented exactly the same amount, so the problem may be that \
                             this new definition has too many spaces in front of it."
                        ),
                    }
                }),
            ]),
        )
    }

    /// The `BadHasType` case of `toOperatorReport`.
    fn bad_has_type(&self, context: &Context<'_>, cur: Position) -> Problem {
        let cons = || {
            fill![
                "Maybe you want",
                green("::"),
                "instead? To put something on the front of a list?"
            ]
        };
        self.at(
            "UNEXPECTED SYMBOL",
            to_region(cur),
            reflow("I was not expecting to run into the \"has type\" symbol here:"),
            match context.def_name() {
                None => cons(),
                Some(name) => stack([
                    cons(),
                    doc::simple_note(&format!(
                        "The single colon is reserved for type annotations and record \
                         types, but I think I am parsing the definition of `{name}` \
                         right now."
                    )),
                    doc::simple_note(&format!(
                        "I may be getting confused by your indentation. Is this supposed \
                         to be part of a type annotation AFTER the `{name}` definition? \
                         If so, the problem may be a bit before the \"has type\" symbol. \
                         I need all definitions to be exactly aligned (with exactly the \
                         same indentation) so the problem may be that this new \
                         definition is indented a bit too much."
                    )),
                ]),
            },
        )
    }

    /// The `BadArrow` case of `toOperatorReport`, which depends on where
    /// the arrow is.
    fn bad_arrow(&self, context: &Context<'_>, cur: Position) -> Problem {
        let in_case = || {
            reflow("I am parsing a `case` expression right now, but this arrow is confusing me:")
        };
        let (before, after) = if context.is_within(Node::Case) {
            (
                in_case(),
                stack([
                    reflow("Maybe the `of` keyword is missing on a previous line?"),
                    note_for_case_error(),
                ]),
            )
        } else if context.is_within(Node::Branch) {
            (
                in_case(),
                stack([
                    reflow(
                        "It makes sense to see arrows around here, so I suspect it is something \
                         earlier. Maybe this pattern is indented a bit farther than the previous \
                         patterns?",
                    ),
                    note_for_case_indent_error(),
                ]),
            )
        } else {
            (
                reflow(
                    "I was partway through parsing an expression when I got stuck on this arrow:",
                ),
                stack([
                    // One string literal upstream, whose newline is a `line`.
                    vcat([
                        Doc::from(
                            "Arrows should only appear in `case` expressions and anonymous \
                             functions.",
                        ),
                        Doc::from("Maybe it was supposed to be a > sign instead?"),
                    ]),
                    doc::simple_note(
                        "The syntax for anonymous functions is (\\x -> x + 1) so the arguments \
                         all appear after the backslash and before the arrow. Maybe a backslash \
                         is missing earlier?",
                    ),
                ]),
            )
        };
        self.at("UNEXPECTED ARROW", self.wider(cur, 2), before, after)
    }

    /// `toFuncReport`.
    fn func(&self, context: &Context<'_>, func: &Func, start: Position) -> Problem {
        let beginning_arrow = || {
            reflow(
                "I just saw the beginning of an anonymous function, so I was expecting to see an \
                 arrow next:",
            )
        };
        let missing_arrow = || {
            fill![
                "The syntax for anonymous functions is",
                dull_yellow("(\\x -> x + 1)"),
                "so I am missing the arrow and the body of the function."
            ]
        };
        let indentation = || {
            doc::simple_note(
                "It is possible that I am confused about indetation! I generally recommend \
                 switching to named functions if the definition cannot fit inline nicely, so \
                 either (1) try to fit the whole anonymous function on one line or (2) break the \
                 whole thing out into a named function. Things tend to be clearer that way!",
            )
        };
        match func {
            Func::FuncSpace(space, cur) => self.space(*space, *cur),
            Func::FuncArg(pattern, cur) => self.pattern(PContext::Arg, pattern, *cur),
            Func::FuncBody(expr, cur) => self.expr(&context.node(Node::Func, start), expr, *cur),
            Func::FuncArrow(cur) => match self.next_keyword(*cur) {
                Some(keyword) => self.reserved(
                    start,
                    *cur,
                    keyword,
                    reflow("I was parsing an anonymous function, but I got stuck here:"),
                    reflow(&format!(
                        "It looks like you are trying to use `{keyword}` as an argument, but it \
                         is a reserved word in this language. Try using a different argument \
                         name!"
                    )),
                ),
                None => self.stuck(
                    "UNFINISHED ANONYMOUS FUNCTION",
                    start,
                    *cur,
                    beginning_arrow(),
                    missing_arrow(),
                ),
            },
            Func::FuncIndentArg(cur) => self.stuck(
                "MISSING ARGUMENT",
                start,
                *cur,
                reflow(
                    "I just saw the beginning of an anonymous function, so I was expecting to see \
                     an argument next:",
                ),
                stack([
                    fill![
                        "Something like",
                        dull_yellow("x"),
                        "or",
                        dull_yellow("name").append(Doc::from(".")),
                        "Anything that starts with a lower case letter!"
                    ],
                    doc::simple_note(
                        "The syntax for anonymous functions is (\\x -> x + 1) where the \
                         backslash is meant to look a bit like a lambda if you squint. This \
                         visual pun seemed like a better idea at the time!",
                    ),
                ]),
            ),
            Func::FuncIndentArrow(cur) => self.stuck(
                "UNFINISHED ANONYMOUS FUNCTION",
                start,
                *cur,
                beginning_arrow(),
                stack([missing_arrow(), indentation()]),
            ),
            Func::FuncIndentBody(cur) => self.stuck(
                "UNFINISHED ANONYMOUS FUNCTION",
                start,
                *cur,
                reflow("I was expecting to see the body of your anonymous function next:"),
                stack([
                    fill![
                        "The syntax for anonymous functions is",
                        dull_yellow("(\\x -> x + 1)"),
                        "so I am missing all the stuff after the arrow!"
                    ],
                    indentation(),
                ]),
            ),
        }
    }
}

impl Node {
    /// What `Start` calls an expression in this node.
    fn a_thing(self) -> &'static str {
        match self {
            Node::Record => "a record",
            Node::Parens => "some parentheses",
            Node::List => "a list",
            Node::Func => "an anonymous function",
            Node::Cond | Node::Then | Node::Else => "an `if` expression",
            Node::Case | Node::Branch => "a `case` expression",
        }
    }
}

fn accessor_example() -> Doc {
    fill![
        "Something like",
        dull_yellow(".name"),
        "or",
        dull_yellow(".price"),
        "that accesses a value from a record."
    ]
}
