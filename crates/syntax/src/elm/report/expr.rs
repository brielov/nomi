//! The reports on expressions: `toExprReport` with the context it
//! carries, `toOperatorReport` and `toFuncReport`. Blocks, collections
//! and patterns have their own modules.

use std::borrow::Cow;

use nomi_base::Compat;
use nomi_base::diagnostic::code::Next;
use nomi_base::diagnostic::doc::{self, fill_sep, reflow, stack, vcat};
use nomi_base::diagnostic::{Doc, Problem};

use super::blocks::{note_for_case_error, note_for_case_indent_error};
use super::pattern::PContext;
use super::{Reporter, dullyellow, green, surroundings, to_region};
use crate::elm::error::{BadOperator, Cursor, Expr, Func};

/// `Context`: what an expression is in, which some reports name.
#[derive(Clone, Copy)]
pub(super) enum Context<'a> {
    /// `InNode`.
    Node(Node, Cursor, &'a Context<'a>),
    /// `InDef`.
    Def(&'a str, Cursor),
    /// `InDestruct`.
    Destruct(Cursor),
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
    /// `getDefName`.
    fn def_name(&self) -> Option<&'a str> {
        match *self {
            Context::Destruct(_) => None,
            Context::Def(name, _) => Some(name),
            Context::Node(_, _, context) => context.def_name(),
        }
    }

    /// `isWithin`, which looks at the innermost node only.
    fn is_within(&self, node: Node) -> bool {
        matches!(*self, Context::Node(actual, _, _) if actual == node)
    }
}

impl Reporter<'_, '_> {
    /// `toExprReport`.
    pub(super) fn expr(&self, context: &Context<'_>, expr: &Expr, start: Cursor) -> Problem {
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
            Expr::OperatorRight(op, cur) => {
                let is_math = ["-", "+", "*", "/", "^"].contains(&&**op);
                self.within(
                    "MISSING EXPRESSION",
                    surroundings(start, *cur),
                    to_region(*cur),
                    reflow(&format!(
                        "I just saw a {op} {}, so I am getting stuck here:",
                        if is_math { "sign" } else { "operator" }
                    )),
                    if is_math {
                        fill_sep(words![
                            "I was expecting to see an expression next. Something like",
                            dullyellow("42"),
                            "or",
                            dullyellow("1000"),
                            "that makes sense with a",
                            Doc::from(&**op),
                            "sign.",
                        ])
                    } else if &**op == "&&" || &**op == "||" {
                        fill_sep(words![
                            "I was expecting to see an expression next. Something like",
                            dullyellow("True"),
                            "or",
                            dullyellow("False"),
                            "that makes sense with boolean logic.",
                        ])
                    } else if &**op == "|>" {
                        reflow("I was expecting to see a function next.")
                    } else if &**op == "<|" {
                        reflow("I was expecting to see an argument next.")
                    } else {
                        reflow("I was expecting to see an expression next.")
                    },
                )
            }
            Expr::OperatorReserved(operator, cur) => self.operator(context, *operator, *cur),
            Expr::Start(cur) => {
                let (context_cur, a_thing): (Cursor, Cow<'_, str>) = match *context {
                    Context::Destruct(c) => (c, "a definition".into()),
                    Context::Def(name, c) => (c, format!("the `{name}` definition").into()),
                    Context::Node(node, c, _) => (c, node.a_thing().into()),
                };
                self.within(
                    "MISSING EXPRESSION",
                    surroundings(context_cur, *cur),
                    to_region(*cur),
                    reflow(&format!(
                        "I am partway through parsing {a_thing}, but I got stuck here:"
                    )),
                    stack([
                        fill_sep(words![
                            "I was expecting to see an expression like",
                            dullyellow("42"),
                            "or",
                            dullyellow("\"hello\"").append(Doc::from(".")),
                            "Once there is something there, I can probably give a more \
                             specific hint!",
                        ]),
                        doc::simple_note(&format!(
                            "This can also happen if {i}run into reserved words like `let` or \
                             `as` unexpectedly. Or if I run into operators in unexpected spots. \
                             Point is, there are a couple ways I can get confused and give sort \
                             of weird advice!",
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
            Expr::Char(char, cur) => self.char(char, *cur),
            Expr::String(string, cur) => self.string(string, *cur),
            Expr::Number(number, cur) => self.number(number, *cur),
            Expr::ShaderEndless(cur) => self.shader_endless(*cur),
            Expr::ShaderNotUtf8(cur) => self.not_utf8(*cur),
            Expr::ShaderProblem(problem, cur) => self.shader_problem(problem, *cur),
            Expr::Space(space, cur) => self.space(*space, *cur),
            Expr::IndentOperatorRight(op, cur) => self.within(
                "MISSING EXPRESSION",
                surroundings(start, *cur),
                to_region(*cur),
                reflow(&format!(
                    "I was expecting to see an expression after this {op} operator:"
                )),
                stack([
                    fill_sep(words![
                        "You can just put anything for now, like",
                        dullyellow("42"),
                        "or",
                        dullyellow("\"hello\"").append(Doc::from(".")),
                        "Once there is something there, I can probably give a more specific hint!",
                    ]),
                    doc::simple_note(&format!(
                        "I may be getting confused by your indentation? The easiest way to make \
                         sure this is not an indentation problem is to put the expression on the \
                         right of the {op} operator on the same line."
                    )),
                ]),
            ),
        }
    }

    /// `toOperatorReport`.
    fn operator(&self, context: &Context<'_>, operator: BadOperator, cur: Cursor) -> Problem {
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
            BadOperator::BadArrow => {
                let (before, after) = if context.is_within(Node::Case) {
                    (
                        reflow(
                            "I am parsing a `case` expression right now, but this arrow is \
                             confusing me:",
                        ),
                        stack([
                            reflow("Maybe the `of` keyword is missing on a previous line?"),
                            note_for_case_error(),
                        ]),
                    )
                } else if context.is_within(Node::Branch) {
                    (
                        reflow(
                            "I am parsing a `case` expression right now, but this arrow is \
                             confusing me:",
                        ),
                        stack([
                            reflow(
                                "It makes sense to see arrows around here, so I suspect it is \
                                 something earlier. Maybe this pattern is indented a bit farther \
                                 than the previous patterns?",
                            ),
                            note_for_case_indent_error(),
                        ]),
                    )
                } else {
                    (
                        reflow(
                            "I was partway through parsing an expression when I got stuck on \
                             this arrow:",
                        ),
                        stack([
                            // One string literal upstream, whose newline is a `line`.
                            vcat([
                                Doc::from(
                                    "Arrows should only appear in `case` expressions and \
                                     anonymous functions.",
                                ),
                                Doc::from("Maybe it was supposed to be a > sign instead?"),
                            ]),
                            doc::simple_note(
                                "The syntax for anonymous functions is (\\x -> x + 1) so the \
                                 arguments all appear after the backslash and before the arrow. \
                                 Maybe a backslash is missing earlier?",
                            ),
                        ]),
                    )
                };
                self.at("UNEXPECTED ARROW", self.wider(cur, 2), before, after)
            }
            BadOperator::BadEquals => self.at(
                "UNEXPECTED EQUALS",
                region,
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
            ),
            BadOperator::BadHasType => {
                let cons = || {
                    fill_sep(words![
                        "Maybe you want",
                        green("::"),
                        "instead? To put something on the front of a list?",
                    ])
                };
                self.at(
                    "UNEXPECTED SYMBOL",
                    region,
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
        }
    }

    /// `toFuncReport`.
    fn func(&self, context: &Context<'_>, func: &Func, start: Cursor) -> Problem {
        let within = |title: &'static str, cur: Cursor, before: &str, after: Doc| {
            self.within(
                title,
                surroundings(start, cur),
                to_region(cur),
                reflow(before),
                after,
            )
        };
        let beginning_arrow = "I just saw the beginning of an anonymous function, so I was \
                               expecting to see an arrow next:";
        let missing_arrow = || {
            fill_sep(words![
                "The syntax for anonymous functions is",
                dullyellow("(\\x -> x + 1)"),
                "so I am missing the arrow and the body of the function.",
            ])
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
            Func::FuncBody(expr, cur) => {
                self.expr(&Context::Node(Node::Func, start, context), expr, *cur)
            }
            Func::FuncArrow(cur) => match self.next(*cur) {
                Next::Keyword(keyword) => self.within(
                    "RESERVED WORD",
                    surroundings(start, *cur),
                    self.keyword_region(*cur, keyword),
                    reflow("I was parsing an anonymous function, but I got stuck here:"),
                    reflow(&format!(
                        "It looks like you are trying to use `{keyword}` as an argument, but it \
                         is a reserved word in this language. Try using a different argument \
                         name!"
                    )),
                ),
                _ => within(
                    "UNFINISHED ANONYMOUS FUNCTION",
                    *cur,
                    beginning_arrow,
                    missing_arrow(),
                ),
            },
            Func::FuncIndentArg(cur) => within(
                "MISSING ARGUMENT",
                *cur,
                "I just saw the beginning of an anonymous function, so I was expecting to see an \
                 argument next:",
                stack([
                    fill_sep(words![
                        "Something like",
                        dullyellow("x"),
                        "or",
                        dullyellow("name").append(Doc::from(".")),
                        "Anything that starts with a lower case letter!",
                    ]),
                    doc::simple_note(
                        "The syntax for anonymous functions is (\\x -> x + 1) where the \
                         backslash is meant to look a bit like a lambda if you squint. This \
                         visual pun seemed like a better idea at the time!",
                    ),
                ]),
            ),
            Func::FuncIndentArrow(cur) => within(
                "UNFINISHED ANONYMOUS FUNCTION",
                *cur,
                beginning_arrow,
                stack([missing_arrow(), indentation()]),
            ),
            Func::FuncIndentBody(cur) => within(
                "UNFINISHED ANONYMOUS FUNCTION",
                *cur,
                "I was expecting to see the body of your anonymous function next:",
                stack([
                    fill_sep(words![
                        "The syntax for anonymous functions is",
                        dullyellow("(\\x -> x + 1)"),
                        "so I am missing all the stuff after the arrow!",
                    ]),
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
    fill_sep(words![
        "Something like",
        dullyellow(".name"),
        "or",
        dullyellow(".price"),
        "that accesses a value from a record.",
    ])
}
