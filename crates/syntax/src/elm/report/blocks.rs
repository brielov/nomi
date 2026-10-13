//! The reports on expressions that hold blocks: `toLetReport` with its
//! definitions and destructurings, `toCaseReport` and `toIfReport`.

use nomi_base::diagnostic::code::Next;
use nomi_base::diagnostic::doc::{self, blue, cyan, dull_yellow, green, reflow, stack, vcat};
use nomi_base::diagnostic::{Doc, Problem};

use super::decl::{Missing, decl_def_note, stuck_in_definition};
use super::expr::{Context, Node};
use super::pattern::PContext;
use super::types::TypeContext;
use super::{Reporter, surroundings};
use crate::elm::error::{Case, Def, Destruct, If, Indent, Let, Position};

impl Reporter<'_, '_> {
    /// `toLetReport`.
    pub(super) fn let_(&self, context: &Context<'_>, let_: &Let, start: Position) -> Problem {
        let partway =
            || reflow("I was partway through parsing a `let` expression, but I got stuck here:");
        match let_ {
            Let::LetSpace(space, cur) => self.space(*space, *cur),
            Let::LetIn(cur) | Let::LetDefAlignment(cur) => self.stuck(
                "LET PROBLEM",
                start,
                *cur,
                partway(),
                stack([
                    fill![
                        "Based on the indentation, I was expecting to see the",
                        cyan("in"),
                        "keyword next. Is there a typo?"
                    ],
                    doc::simple_note(
                        "This can also happen if you are trying to define another value within \
                         the `let` but it is not indented enough. Make sure each definition has \
                         exactly the same amount of spaces before it. They should line up \
                         exactly!",
                    ),
                ]),
            ),
            Let::LetDefName(cur) => match self.next_keyword(*cur) {
                Some(keyword) => self.reserved(
                    start,
                    *cur,
                    keyword,
                    partway(),
                    reflow(&format!(
                        "It looks like you are trying to use `{keyword}` as a variable name, but \
                         it is a reserved word! Try using a different name instead."
                    )),
                ),
                None => self.unfinished_let(
                    *cur,
                    start,
                    reflow("I was expecting the name of a definition next."),
                ),
            },
            Let::LetDef(name, def, cur) => self.let_def(name, def, *cur),
            Let::LetDestruct(destruct, cur) => self.destruct(destruct, *cur),
            Let::LetBody(expr, cur) => self.expr(context, expr, *cur),
            Let::LetIndentDef(cur) => self.unfinished_let(
                *cur,
                start,
                reflow("I was expecting a value to be defined here."),
            ),
            Let::LetIndentIn(cur) => self.unfinished_let(
                *cur,
                start,
                fill![
                    "I was expecting to see the",
                    cyan("in"),
                    "keyword next. Or maybe more of that expression?"
                ],
            ),
            Let::LetIndentBody(cur) => self.unfinished_let(
                *cur,
                start,
                reflow(
                    "I was expecting an expression next. Tell me what should happen with the \
                     value you just defined!",
                ),
            ),
        }
    }

    /// `toUnfinishLetReport`.
    fn unfinished_let(&self, cur: Position, start: Position, message: Doc) -> Problem {
        self.stuck(
            "UNFINISHED LET",
            start,
            cur,
            reflow("I was partway through parsing a `let` expression, but I got stuck here:"),
            stack([
                message,
                doc::simple_note("Here is an example with a valid `let` expression for reference:"),
                vcat([
                    reflow("viewPerson person ="),
                    cyan("let").indent(2),
                    reflow("fullName =").indent(4),
                    fill![
                        "person.firstName ++",
                        dull_yellow("\" \""),
                        "++ person.lastName"
                    ]
                    .indent(6),
                    cyan("in").indent(2),
                    reflow("div [] [ text fullName ]").indent(2),
                ])
                .indent(4),
                reflow(
                    "Here we defined a `viewPerson` function that turns a person into some HTML. \
                     We use a `let` expression to define the `fullName` we want to show. Notice \
                     the indentation! The `fullName` is indented more than the `let` keyword, and \
                     the actual value of `fullName` is indented a bit more than that. That is \
                     important!",
                ),
            ]),
        )
    }

    /// `toLetDefReport`.
    fn let_def(&self, name: &str, def: &Def, start: Position) -> Problem {
        match def {
            Def::DefSpace(space, cur) => self.space(*space, *cur),
            Def::DefType(tipe, cur) => self.tipe(&TypeContext::Annotation(name), tipe, *cur),
            Def::DefNameRepeat(cur) => self.stuck(
                "EXPECTING DEFINITION",
                start,
                *cur,
                reflow(&format!(
                    "I just saw the type annotation for `{name}` so I was expecting to see its \
                     definition here:"
                )),
                stack([
                    reflow(
                        "Type annotations always appear directly above the relevant definition, \
                         without anything else in between.",
                    ),
                    decl_def_note(),
                ]),
            ),
            Def::DefNameMatch(def_name, cur) => self.name_mismatch(name, def_name, *cur, start),
            Def::DefArg(pattern, cur) => self.pattern(PContext::Arg, pattern, *cur),
            Def::DefEquals(cur) => self.def_equals(name, *cur, start),
            Def::DefBody(expr, cur) => self.expr(&Context::Def(name, start), expr, *cur),
            Def::DefIndentEquals(cur) => {
                self.unfinished_definition(name, start, *cur, Missing::Equals)
            }
            Def::DefIndentType(cur) => self.unfinished_definition(name, start, *cur, Missing::Type),
            Def::DefIndentBody(cur) => self.unfinished_definition(name, start, *cur, Missing::Body),
            Def::DefAlignment(indent, cur) => {
                let offset = self.distance_to_indent(*indent, *cur);
                self.stuck(
                    "PROBLEM IN DEFINITION",
                    start,
                    *cur,
                    stuck_in_definition(name),
                    reflow(&format!(
                        "I just saw a type annotation indented {indent} spaces, so I was \
                         expecting to see the corresponding definition next with the exact same \
                         amount of indentation. It looks like this line needs {offset} more {}?",
                        if offset == 1 { "space" } else { "spaces" }
                    )),
                )
            }
        }
    }

    /// `distanceToIndent`: `indent - col`, in the width of a column.
    fn distance_to_indent(&self, indent: Indent, cur: Position) -> u32 {
        self.compat.wrap(indent.wrapping_sub(cur.col))
    }

    /// `toLetDestructReport`.
    fn destruct(&self, destruct: &Destruct, start: Position) -> Problem {
        let expecting_equals = || {
            reflow(
                "I was expecting to see an equals sign next, followed by an expression telling \
                 me what to compute.",
            )
        };
        let stuck_trying = || reflow("I got stuck trying to parse this definition:");
        match destruct {
            Destruct::DestructSpace(space, cur) => self.space(*space, *cur),
            Destruct::DestructPattern(pattern, cur) => self.pattern(PContext::Let, pattern, *cur),
            Destruct::DestructEquals(cur) => self.stuck(
                "PROBLEM IN DEFINITION",
                start,
                *cur,
                stuck_trying(),
                match self.next(*cur) {
                    Next::Operator(op) if op == ":" => stack([
                        expecting_equals(),
                        doc::simple_note(
                            "It looks like you may be trying to write a type annotation? It is \
                             not possible to add type annotations on destructuring definitions \
                             like this. You can assign a name to the overall structure, put a \
                             type annotation on that, and then destructure separately though.",
                        ),
                    ]),
                    Next::Keyword(_)
                    | Next::Operator(_)
                    | Next::Close(_)
                    | Next::Upper(..)
                    | Next::Lower(..)
                    | Next::Other(_) => expecting_equals(),
                },
            ),
            Destruct::DestructBody(expr, cur) => self.expr(&Context::Destruct(start), expr, *cur),
            Destruct::DestructIndentEquals(cur) => self.stuck(
                "UNFINISHED DEFINITION",
                start,
                *cur,
                stuck_trying(),
                expecting_equals(),
            ),
            Destruct::DestructIndentBody(cur) => self.stuck(
                "UNFINISHED DEFINITION",
                start,
                *cur,
                reflow("I got stuck while parsing this definition:"),
                reflow("I was expecting to see an expression next. What is it equal to?"),
            ),
        }
    }

    // CASE

    /// `toCaseReport`.
    pub(super) fn case(&self, context: &Context<'_>, case: &Case, start: Position) -> Problem {
        match case {
            Case::CaseSpace(space, cur) => self.space(*space, *cur),
            Case::CaseOf(cur) | Case::CaseIndentOf(cur) => self.unfinished_case(
                *cur,
                start,
                fill![
                    "I was expecting to see the",
                    dull_yellow("of"),
                    "keyword next."
                ],
            ),
            Case::CasePattern(pattern, cur) => self.pattern(PContext::Case, pattern, *cur),
            Case::CaseArrow(cur) => self.case_arrow(start, *cur),
            Case::CaseExpr(expr, cur) => self.expr(&context.node(Node::Case, start), expr, *cur),
            Case::CaseBranch(expr, cur) => {
                self.expr(&context.node(Node::Branch, start), expr, *cur)
            }
            Case::CaseIndentExpr(cur) => self.unfinished_case(
                *cur,
                start,
                reflow("I was expecting to see a expression next."),
            ),
            Case::CaseIndentPattern(cur) => self.unfinished_case(
                *cur,
                start,
                reflow("I was expecting to see a pattern next."),
            ),
            Case::CaseIndentArrow(cur) => self.unfinished_case(
                *cur,
                start,
                fill![
                    "I just saw a pattern, so I was expecting to see a",
                    dull_yellow("->"),
                    "next."
                ],
            ),
            Case::CaseIndentBranch(cur) => self.unfinished_case(
                *cur,
                start,
                reflow(
                    "I was expecting to see an expression next. What should I do when I run into \
                     this particular pattern?",
                ),
            ),
            Case::CasePatternAlignment(indent, cur) => self.unfinished_case(
                *cur,
                start,
                reflow(&format!(
                    "I suspect this is a pattern that is not indented far enough? ({indent} \
                     spaces)"
                )),
            ),
        }
    }

    /// `CaseArrow`, by what is there.
    fn case_arrow(&self, start: Position, cur: Position) -> Problem {
        let partway =
            || reflow("I am partway through parsing a `case` expression, but I got stuck here:");
        match self.next(cur) {
            Next::Keyword(keyword) => self.reserved(
                start,
                cur,
                keyword,
                partway(),
                reflow(&format!(
                    "It looks like you are trying to use `{keyword}` in one of your patterns, \
                     but it is a reserved word. Try using a different name?"
                )),
            ),
            Next::Operator(op) if op == ":" => self.stuck(
                "UNEXPECTED OPERATOR",
                start,
                cur,
                partway(),
                fill![
                    "I am seeing",
                    dull_yellow(":"),
                    "but maybe you want",
                    green("::"),
                    "instead? For pattern matching on lists?"
                ],
            ),
            Next::Operator(op) if op == "=" => self.stuck(
                "UNEXPECTED OPERATOR",
                start,
                cur,
                partway(),
                fill![
                    "I am seeing",
                    dull_yellow("="),
                    "but maybe you want",
                    green("->"),
                    "instead?"
                ],
            ),
            Next::Operator(_)
            | Next::Close(_)
            | Next::Upper(..)
            | Next::Lower(..)
            | Next::Other(_) => self.stuck(
                "MISSING ARROW",
                start,
                cur,
                partway(),
                stack([
                    reflow("I was expecting to see an arrow next."),
                    note_for_case_indent_error(),
                ]),
            ),
        }
    }

    /// `toUnfinishCaseReport`.
    fn unfinished_case(&self, cur: Position, start: Position, message: Doc) -> Problem {
        self.stuck(
            "UNFINISHED CASE",
            start,
            cur,
            reflow("I was partway through parsing a `case` expression, but I got stuck here:"),
            stack([message, note_for_case_error()]),
        )
    }

    // IF

    /// `toIfReport`.
    pub(super) fn if_(&self, context: &Context<'_>, if_: &If, start: Position) -> Problem {
        let more =
            || reflow("I was expecting to see more of this `if` expression, but I got stuck here:");
        let keyword_next = |keyword: &'static str| {
            fill!["I was expecting to see the", cyan(keyword), "keyword next."]
        };
        let confused = || {
            doc::simple_note(
                "I can be confused by indentation. Maybe something is not indented enough?",
            )
        };
        let maybe_not_filled =
            || reflow("I was expecting to see an expression next. Maybe it is not filled in yet?");
        match if_ {
            If::IfSpace(space, cur) => self.space(*space, *cur),
            If::IfThen(cur) => {
                self.stuck("UNFINISHED IF", start, *cur, more(), keyword_next("then"))
            }
            If::IfElse(cur) => {
                self.stuck("UNFINISHED IF", start, *cur, more(), keyword_next("else"))
            }
            If::IfElseBranchStart(cur) => self.stuck(
                "UNFINISHED IF",
                start,
                *cur,
                reflow("I just saw the start of an `else` branch, but then I got stuck here:"),
                maybe_not_filled(),
            ),
            If::IfCondition(expr, cur) => self.expr(&context.node(Node::Cond, start), expr, *cur),
            If::IfThenBranch(expr, cur) => self.expr(&context.node(Node::Then, start), expr, *cur),
            If::IfElseBranch(expr, cur) => self.expr(&context.node(Node::Else, start), expr, *cur),
            If::IfIndentCondition(cur) => self.stuck(
                "UNFINISHED IF",
                start,
                *cur,
                more(),
                stack([
                    fill![
                        "I was expecting to see an expression like",
                        dull_yellow("x < 0"),
                        "that evaluates to True or False."
                    ],
                    confused(),
                ]),
            ),
            If::IfIndentThen(cur) => self.stuck(
                "UNFINISHED IF",
                start,
                *cur,
                more(),
                stack([keyword_next("then"), confused()]),
            ),
            If::IfIndentThenBranch(cur) => self.stuck(
                "UNFINISHED IF",
                start,
                *cur,
                reflow("I got stuck after the start of this `then` branch:"),
                stack([
                    maybe_not_filled(),
                    doc::simple_note(
                        "I can be confused by indentation, so if the `then` branch is already \
                         present, it may not be indented enough for me to recognize it.",
                    ),
                ]),
            ),
            If::IfIndentElseBranch(cur) => self.stuck(
                "UNFINISHED IF",
                start,
                *cur,
                reflow("I got stuck after the start of this `else` branch:"),
                stack([
                    maybe_not_filled(),
                    doc::simple_note(
                        "I can be confused by indentation, so if the `else` branch is already \
                         present, it may not be indented enough for me to recognize it.",
                    ),
                ]),
            ),
            If::IfIndentElse(cur) => self.missing_else(start, *cur),
        }
    }

    /// `IfIndentElse`: an `else` not indented enough, or none.
    fn missing_else(&self, start: Position, cur: Position) -> Problem {
        match (self.source).next_line_starts_with_keyword("else", cur, self.compat) {
            Some(else_cur) => self.within(
                "WEIRD ELSE BRANCH",
                surroundings(start, else_cur),
                self.wider(else_cur, 4),
                reflow("I was partway through an `if` expression when I got stuck here:"),
                fill![
                    "I think this",
                    cyan("else"),
                    "keyword needs to be indented more. Try adding some spaces before it."
                ],
            ),
            None => self.stuck(
                "UNFINISHED IF",
                start,
                cur,
                reflow("I was expecting to see an `else` branch after this:"),
                fill![
                    "I know what to do when the condition is True, but what happens when it is \
                     False? Add an",
                    cyan("else"),
                    "branch to handle that scenario!"
                ],
            ),
        }
    }
}

/// `noteForCaseError`.
pub(super) fn note_for_case_error() -> Doc {
    stack([
        doc::simple_note("Here is an example of a valid `case` expression for reference."),
        case_example(),
        reflow(
            "Notice the indentation. Each pattern is aligned, and each branch is indented a bit \
             more than the corresponding pattern. That is important!",
        ),
    ])
}

/// `noteForCaseIndentError`.
pub(super) fn note_for_case_indent_error() -> Doc {
    stack([
        doc::simple_note(
            "Sometimes I get confused by indentation, so try to make your `case` look something \
             like this:",
        ),
        case_example(),
        reflow(
            "Notice the indentation! Patterns are aligned with each other. Same indentation. The \
             expressions after each arrow are all indented a bit more than the patterns. That is \
             important!",
        ),
    ])
}

fn case_example() -> Doc {
    vcat([
        fill![cyan("case"), "maybeWidth", cyan("of")].indent(4),
        fill![blue("Just"), "width ->"].indent(6),
        fill!["width +", dull_yellow("200")].indent(8),
        Doc::empty(),
        fill![blue("Nothing"), "->"].indent(6),
        dull_yellow("400").indent(8),
    ])
}
