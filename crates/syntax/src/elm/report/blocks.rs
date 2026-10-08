//! The reports on expressions that hold blocks: `toLetReport` with its
//! definitions and destructurings, `toCaseReport` and `toIfReport`.

use nomi_base::diagnostic::code::Next;
use nomi_base::diagnostic::doc::{self, fill_sep, reflow, stack, vcat};
use nomi_base::diagnostic::{Doc, Problem};

use super::decl::decl_def_note;
use super::expr::{Context, Node};
use super::pattern::PContext;
use super::types::TypeContext;
use super::{Reporter, blue, cursor, cyan, dullyellow, green, surroundings, to_region};
use crate::elm::error::{Case, Cursor, Def, Destruct, If, Indent, Let};

impl Reporter<'_, '_> {
    /// `toLetReport`.
    pub(super) fn let_(&self, context: &Context<'_>, let_: &Let, start: Cursor) -> Problem {
        let partway =
            || reflow("I was partway through parsing a `let` expression, but I got stuck here:");
        let expecting_in = |cur: Cursor| {
            self.within(
                "LET PROBLEM",
                surroundings(start, cur),
                to_region(cur),
                partway(),
                stack([
                    fill_sep(words![
                        "Based on the indentation, I was expecting to see the",
                        cyan("in"),
                        "keyword next. Is there a typo?",
                    ]),
                    doc::simple_note(
                        "This can also happen if you are trying to define another value within \
                         the `let` but it is not indented enough. Make sure each definition has \
                         exactly the same amount of spaces before it. They should line up \
                         exactly!",
                    ),
                ]),
            )
        };
        match let_ {
            Let::LetSpace(space, cur) => self.space(*space, *cur),
            Let::LetIn(cur) | Let::LetDefAlignment(_, cur) => expecting_in(*cur),
            Let::LetDefName(cur) => match self.next(*cur) {
                Next::Keyword(keyword) => self.within(
                    "RESERVED WORD",
                    surroundings(start, *cur),
                    self.keyword_region(*cur, keyword),
                    partway(),
                    reflow(&format!(
                        "It looks like you are trying to use `{keyword}` as a variable name, but \
                         it is a reserved word! Try using a different name instead."
                    )),
                ),
                _ => self.unfinished_let(
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
                fill_sep(words![
                    "I was expecting to see the",
                    cyan("in"),
                    "keyword next. Or maybe more of that expression?",
                ]),
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
    fn unfinished_let(&self, cur: Cursor, start: Cursor, message: Doc) -> Problem {
        self.within(
            "UNFINISHED LET",
            surroundings(start, cur),
            to_region(cur),
            reflow("I was partway through parsing a `let` expression, but I got stuck here:"),
            stack([
                message,
                doc::simple_note("Here is an example with a valid `let` expression for reference:"),
                vcat([
                    fill_sep(words!["viewPerson person ="]),
                    fill_sep(words![cyan("let")]).indent(2),
                    fill_sep(words!["fullName ="]).indent(4),
                    fill_sep(words![
                        "person.firstName ++",
                        dullyellow("\" \""),
                        "++ person.lastName",
                    ])
                    .indent(6),
                    fill_sep(words![cyan("in")]).indent(2),
                    fill_sep(words!["div [] [ text fullName ]"]).indent(2),
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
    fn let_def(&self, name: &str, def: &Def, start: Cursor) -> Problem {
        let stuck = |cur: Cursor, title: &'static str, before: String, after: Doc| {
            self.within(
                title,
                surroundings(start, cur),
                to_region(cur),
                reflow(&before),
                after,
            )
        };
        let stuck_definition = || format!("I got stuck while parsing the `{name}` definition:");
        match def {
            Def::DefSpace(space, cur) => self.space(*space, *cur),
            Def::DefType(tipe, cur) => self.tipe(&TypeContext::Annotation(name), tipe, *cur),
            Def::DefNameRepeat(cur) => stuck(
                *cur,
                "EXPECTING DEFINITION",
                format!(
                    "I just saw the type annotation for `{name}` so I was expecting to see its \
                     definition here:"
                ),
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
            Def::DefIndentEquals(cur) => stuck(
                *cur,
                "UNFINISHED DEFINITION",
                stuck_definition(),
                stack([
                    reflow("I was expecting to see an argument or an equals sign next."),
                    decl_def_note(),
                ]),
            ),
            Def::DefIndentType(cur) => stuck(
                *cur,
                "UNFINISHED DEFINITION",
                format!("I got stuck while parsing the `{name}` type annotation:"),
                stack([
                    reflow("I just saw a colon, so I am expecting to see a type next."),
                    decl_def_note(),
                ]),
            ),
            Def::DefIndentBody(cur) => stuck(
                *cur,
                "UNFINISHED DEFINITION",
                stuck_definition(),
                stack([
                    reflow("I was expecting to see an expression next. What is it equal to?"),
                    decl_def_note(),
                ]),
            ),
            Def::DefAlignment(indent, cur) => {
                let offset = self.distance_to_indent(*indent, *cur);
                stuck(
                    *cur,
                    "PROBLEM IN DEFINITION",
                    stuck_definition(),
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
    fn distance_to_indent(&self, indent: Indent, cur: Cursor) -> u32 {
        let distance = indent.wrapping_sub(cur.col);
        self.compat
            .column_wrap()
            .map_or(distance, |wrap| distance % wrap)
    }

    /// `toLetDestructReport`.
    fn destruct(&self, destruct: &Destruct, start: Cursor) -> Problem {
        let stuck = |cur: Cursor, title: &'static str, before: &str, after: Doc| {
            self.within(
                title,
                surroundings(start, cur),
                to_region(cur),
                reflow(before),
                after,
            )
        };
        let expecting_equals = || {
            reflow(
                "I was expecting to see an equals sign next, followed by an expression telling \
                 me what to compute.",
            )
        };
        match destruct {
            Destruct::DestructSpace(space, cur) => self.space(*space, *cur),
            Destruct::DestructPattern(pattern, cur) => self.pattern(PContext::Let, pattern, *cur),
            Destruct::DestructEquals(cur) => stuck(
                *cur,
                "PROBLEM IN DEFINITION",
                "I got stuck trying to parse this definition:",
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
                    _ => expecting_equals(),
                },
            ),
            Destruct::DestructBody(expr, cur) => self.expr(&Context::Destruct(start), expr, *cur),
            Destruct::DestructIndentEquals(cur) => stuck(
                *cur,
                "UNFINISHED DEFINITION",
                "I got stuck trying to parse this definition:",
                expecting_equals(),
            ),
            Destruct::DestructIndentBody(cur) => stuck(
                *cur,
                "UNFINISHED DEFINITION",
                "I got stuck while parsing this definition:",
                reflow("I was expecting to see an expression next. What is it equal to?"),
            ),
        }
    }

    // CASE

    /// `toCaseReport`.
    pub(super) fn case(&self, context: &Context<'_>, case: &Case, start: Cursor) -> Problem {
        let partway =
            || reflow("I am partway through parsing a `case` expression, but I got stuck here:");
        let expecting_of = || {
            fill_sep(words![
                "I was expecting to see the",
                dullyellow("of"),
                "keyword next.",
            ])
        };
        match case {
            Case::CaseSpace(space, cur) => self.space(*space, *cur),
            Case::CaseOf(cur) | Case::CaseIndentOf(cur) => {
                self.unfinished_case(*cur, start, expecting_of())
            }
            Case::CasePattern(pattern, cur) => self.pattern(PContext::Case, pattern, *cur),
            Case::CaseArrow(cur) => {
                let operator = |instead: Doc| {
                    self.within(
                        "UNEXPECTED OPERATOR",
                        surroundings(start, *cur),
                        to_region(*cur),
                        partway(),
                        instead,
                    )
                };
                match self.next(*cur) {
                    Next::Keyword(keyword) => self.within(
                        "RESERVED WORD",
                        surroundings(start, *cur),
                        self.keyword_region(*cur, keyword),
                        partway(),
                        reflow(&format!(
                            "It looks like you are trying to use `{keyword}` in one of your \
                             patterns, but it is a reserved word. Try using a different name?"
                        )),
                    ),
                    Next::Operator(op) if op == ":" => operator(fill_sep(words![
                        "I am seeing",
                        dullyellow(":"),
                        "but maybe you want",
                        green("::"),
                        "instead? For pattern matching on lists?",
                    ])),
                    Next::Operator(op) if op == "=" => operator(fill_sep(words![
                        "I am seeing",
                        dullyellow("="),
                        "but maybe you want",
                        green("->"),
                        "instead?",
                    ])),
                    _ => self.within(
                        "MISSING ARROW",
                        surroundings(start, *cur),
                        to_region(*cur),
                        partway(),
                        stack([
                            reflow("I was expecting to see an arrow next."),
                            note_for_case_indent_error(),
                        ]),
                    ),
                }
            }
            Case::CaseExpr(expr, cur) => {
                self.expr(&Context::Node(Node::Case, start, context), expr, *cur)
            }
            Case::CaseBranch(expr, cur) => {
                self.expr(&Context::Node(Node::Branch, start, context), expr, *cur)
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
                fill_sep(words![
                    "I just saw a pattern, so I was expecting to see a",
                    dullyellow("->"),
                    "next.",
                ]),
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

    /// `toUnfinishCaseReport`.
    fn unfinished_case(&self, cur: Cursor, start: Cursor, message: Doc) -> Problem {
        self.within(
            "UNFINISHED CASE",
            surroundings(start, cur),
            to_region(cur),
            reflow("I was partway through parsing a `case` expression, but I got stuck here:"),
            stack([message, note_for_case_error()]),
        )
    }

    // IF

    /// `toIfReport`.
    pub(super) fn if_(&self, context: &Context<'_>, if_: &If, start: Cursor) -> Problem {
        let unfinished = |cur: Cursor, before: &str, after: Doc| {
            self.within(
                "UNFINISHED IF",
                surroundings(start, cur),
                to_region(cur),
                reflow(before),
                after,
            )
        };
        let more = "I was expecting to see more of this `if` expression, but I got stuck here:";
        let keyword_next = |keyword: &'static str| {
            fill_sep(words![
                "I was expecting to see the",
                cyan(keyword),
                "keyword next.",
            ])
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
            If::IfThen(cur) => unfinished(*cur, more, keyword_next("then")),
            If::IfElse(cur) => unfinished(*cur, more, keyword_next("else")),
            If::IfElseBranchStart(cur) => unfinished(
                *cur,
                "I just saw the start of an `else` branch, but then I got stuck here:",
                maybe_not_filled(),
            ),
            If::IfCondition(expr, cur) => {
                self.expr(&Context::Node(Node::Cond, start, context), expr, *cur)
            }
            If::IfThenBranch(expr, cur) => {
                self.expr(&Context::Node(Node::Then, start, context), expr, *cur)
            }
            If::IfElseBranch(expr, cur) => {
                self.expr(&Context::Node(Node::Else, start, context), expr, *cur)
            }
            If::IfIndentCondition(cur) => unfinished(
                *cur,
                more,
                stack([
                    fill_sep(words![
                        "I was expecting to see an expression like",
                        dullyellow("x < 0"),
                        "that evaluates to True or False.",
                    ]),
                    confused(),
                ]),
            ),
            If::IfIndentThen(cur) => {
                unfinished(*cur, more, stack([keyword_next("then"), confused()]))
            }
            If::IfIndentThenBranch(cur) => unfinished(
                *cur,
                "I got stuck after the start of this `then` branch:",
                stack([
                    maybe_not_filled(),
                    doc::simple_note(
                        "I can be confused by indentation, so if the `then` branch is already \
                         present, it may not be indented enough for me to recognize it.",
                    ),
                ]),
            ),
            If::IfIndentElseBranch(cur) => unfinished(
                *cur,
                "I got stuck after the start of this `else` branch:",
                stack([
                    maybe_not_filled(),
                    doc::simple_note(
                        "I can be confused by indentation, so if the `else` branch is already \
                         present, it may not be indented enough for me to recognize it.",
                    ),
                ]),
            ),
            If::IfIndentElse(cur) => match (self.source).next_line_starts_with_keyword(
                "else",
                super::position(*cur),
                self.compat,
            ) {
                Some(else_pos) => {
                    let else_cur = cursor(else_pos);
                    self.within(
                        "WEIRD ELSE BRANCH",
                        surroundings(start, else_cur),
                        self.wider(else_cur, 4),
                        reflow("I was partway through an `if` expression when I got stuck here:"),
                        fill_sep(words![
                            "I think this",
                            cyan("else"),
                            "keyword needs to be indented more. Try adding some spaces before it.",
                        ]),
                    )
                }
                None => unfinished(
                    *cur,
                    "I was expecting to see an `else` branch after this:",
                    stack([fill_sep(words![
                        "I know what to do when the condition is True, but what happens when it \
                         is False? Add an",
                        cyan("else"),
                        "branch to handle that scenario!",
                    ])]),
                ),
            },
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
        fill_sep(words![cyan("case"), "maybeWidth", cyan("of")]).indent(4),
        fill_sep(words![blue("Just"), "width ->"]).indent(6),
        fill_sep(words!["width +", dullyellow("200")]).indent(8),
        Doc::empty(),
        fill_sep(words![blue("Nothing"), "->"]).indent(6),
        fill_sep(words![dullyellow("400")]).indent(8),
    ])
}
