//! The reports on expressions that collect others: `toRecordReport`,
//! `toTupleReport` and `toListReport`.

use nomi_base::diagnostic::code::{Bracket, Next};
use nomi_base::diagnostic::doc::{self, dull_yellow, green, reflow, stack, vcat};
use nomi_base::diagnostic::{Doc, Problem};

use super::expr::{Context, Node};
use super::{
    Reporter, closing_before_this, closing_curly_next, closing_paren_note, convention,
    earlier_trouble_note, reserved_field, try_adding,
};
use crate::elm::error::{BadOperator, Expr, List, Position, Record, Tuple};

impl Reporter<'_, '_> {
    /// `toRecordReport`.
    #[expect(
        clippy::too_many_lines,
        reason = "upstream's `toRecordReport`, one message per error"
    )]
    pub(super) fn record(
        &self,
        context: &Context<'_>,
        record: &Record,
        start: Position,
    ) -> Problem {
        let partway = || reflow("I am partway through parsing a record, but I got stuck here:");
        match record {
            Record::RecordOpen(cur) => match self.next_keyword(*cur) {
                Some(keyword) => self.reserved(
                    start,
                    *cur,
                    keyword,
                    reflow("I just started parsing a record, but I got stuck on this field name:"),
                    reserved_field(keyword),
                ),
                None => self.stuck(
                    "PROBLEM IN RECORD",
                    start,
                    *cur,
                    reflow("I just started parsing a record, but I got stuck here:"),
                    expecting_field("a"),
                ),
            },
            Record::RecordEnd(cur) => self.stuck(
                "PROBLEM IN RECORD",
                start,
                *cur,
                partway(),
                closing_before_this(Bracket::Curly),
            ),
            Record::RecordField(cur) => self.record_field(start, *cur),
            Record::RecordEquals(cur) => self.stuck(
                "PROBLEM IN RECORD",
                start,
                *cur,
                partway(),
                stack([
                    fill![
                        "I just saw a field name, so I was expecting to see an equals sign next. \
                         So try putting an",
                        green("="),
                        "sign here?"
                    ],
                    note_for_record_error(),
                ]),
            ),
            Record::RecordExpr(expr, cur) => {
                self.expr(&context.node(Node::Record, start), expr, *cur)
            }
            Record::RecordSpace(space, cur) => self.space(*space, *cur),
            Record::RecordIndentOpen(cur) => self.stuck(
                "UNFINISHED RECORD",
                start,
                *cur,
                reflow(
                    "I just saw the opening curly brace of a record, but then I got stuck here:",
                ),
                stack([
                    fill![
                        "I am expecting a record like",
                        dull_yellow("{ x = 3, y = 4 }"),
                        "here. Try defining some fields of your own?"
                    ],
                    note_for_record_indent_error(),
                ]),
            ),
            Record::RecordIndentEnd(cur) => {
                let partway = "I was partway through parsing a record, but I got stuck here:";
                self.curly_needs_indent(start, *cur, partway, note_for_record_error)
                    .unwrap_or_else(|| {
                        self.stuck(
                            "UNFINISHED RECORD",
                            start,
                            *cur,
                            reflow(partway),
                            stack([closing_curly_next(), note_for_record_indent_error()]),
                        )
                    })
            }
            Record::RecordIndentField(cur) => self.stuck(
                "UNFINISHED RECORD",
                start,
                *cur,
                reflow(
                    "I am partway through parsing a record, but I got stuck after that last comma:",
                ),
                stack([
                    reflow(
                        "Trailing commas are not allowed in records, so the fix may be to delete \
                         that last comma? Or maybe you were in the middle of defining an \
                         additional field?",
                    ),
                    note_for_record_error(),
                ]),
            ),
            Record::RecordIndentEquals(cur) => self.stuck(
                "UNFINISHED RECORD",
                start,
                *cur,
                reflow(
                    "I am partway through parsing a record. I just saw a record field, so I was \
                     expecting to see an equals sign next:",
                ),
                stack([
                    fill!["Try putting an", green("="), "followed by an expression?"],
                    note_for_record_indent_error(),
                ]),
            ),
            Record::RecordIndentExpr(cur) => self.stuck(
                "UNFINISHED RECORD",
                start,
                *cur,
                reflow(
                    "I am partway through parsing a record, and I was expecting to run into an \
                     expression next:",
                ),
                stack([
                    fill![
                        "Try putting something like",
                        dull_yellow("42"),
                        "or",
                        dull_yellow("\"hello\""),
                        "for now?"
                    ],
                    note_for_record_indent_error(),
                ]),
            ),
        }
    }

    /// `RecordField`, by what is there.
    fn record_field(&self, start: Position, cur: Position) -> Problem {
        let partway = || reflow("I am partway through parsing a record, but I got stuck here:");
        match self.next(cur) {
            Next::Keyword(keyword) => self.reserved(
                start,
                cur,
                keyword,
                reflow(
                    "I am partway through parsing a record, but I got stuck on this field name:",
                ),
                reserved_field(keyword),
            ),
            Next::Other(Some(',')) => self.stuck(
                "EXTRA COMMA",
                start,
                cur,
                partway(),
                stack([
                    reflow("I am seeing two commas in a row. This is the second one!"),
                    reflow("Just delete one of the commas and you should be all set!"),
                    note_for_record_error(),
                ]),
            ),
            Next::Close(Bracket::Curly) => self.stuck(
                "EXTRA COMMA",
                start,
                cur,
                partway(),
                stack([
                    reflow(
                        "Trailing commas are not allowed in records. Try deleting the comma that \
                         appears before this closing curly brace.",
                    ),
                    note_for_record_error(),
                ]),
            ),
            Next::Operator(_)
            | Next::Close(_)
            | Next::Upper(..)
            | Next::Lower(..)
            | Next::Other(_) => self.stuck(
                "PROBLEM IN RECORD",
                start,
                cur,
                partway(),
                expecting_field("another"),
            ),
        }
    }

    /// `toTupleReport`.
    pub(super) fn tuple(&self, context: &Context<'_>, tuple: &Tuple, start: Position) -> Problem {
        let not_indented = || {
            doc::simple_note(
                "I can get confused by indentation in cases like this, so maybe you have an \
                 expression but it is not indented enough?",
            )
        };
        match tuple {
            Tuple::TupleExpr(expr, cur) => {
                self.expr(&context.node(Node::Parens, start), expr, *cur)
            }
            Tuple::TupleSpace(space, cur) => self.space(*space, *cur),
            Tuple::TupleEnd(cur) => self.stuck(
                "UNFINISHED PARENTHESES",
                start,
                *cur,
                reflow("I was expecting to see a closing parentheses next, but I got stuck here:"),
                stack([try_adding(Bracket::Paren, '?'), earlier_trouble_note()]),
            ),
            Tuple::TupleOperatorClose(cur) => self.stuck(
                "UNFINISHED OPERATOR FUNCTION",
                start,
                *cur,
                reflow("I was expecting a closing parenthesis here:"),
                stack([
                    try_adding(Bracket::Paren, '!'),
                    doc::simple_note(
                        "I think I am parsing an operator function right now, so I am expecting \
                         to see something like (+) or (&&) where an operator is surrounded by \
                         parentheses with no extra spaces.",
                    ),
                ]),
            ),
            Tuple::TupleOperatorReserved(operator, cur) => self.stuck(
                "UNEXPECTED SYMBOL",
                start,
                *cur,
                reflow("I ran into an unexpected symbol here:"),
                operator_function_hint(*operator),
            ),
            Tuple::TupleIndentExpr1(cur) => self.stuck(
                "UNFINISHED PARENTHESES",
                start,
                *cur,
                reflow(
                    "I just saw an open parenthesis, so I was expecting to see an expression next.",
                ),
                stack([
                    fill![
                        "Something like",
                        dull_yellow("(4 + 5)"),
                        "or",
                        dull_yellow("(String.reverse \"desserts\")").append(Doc::from(".")),
                        "Anything where you are putting parentheses around normal expressions."
                    ],
                    not_indented(),
                ]),
            ),
            Tuple::TupleIndentExprN(cur) => self.stuck(
                "UNFINISHED TUPLE",
                start,
                *cur,
                reflow(
                    "I think I am in the middle of parsing a tuple. I just saw a comma, so I was \
                     expecting to see an expression next.",
                ),
                stack([
                    fill![
                        "A tuple looks like",
                        dull_yellow("(3,4)"),
                        "or",
                        dull_yellow("(\"Tom\",42)").append(Doc::from(",")),
                        "so I think there is an expression missing here?"
                    ],
                    not_indented(),
                ]),
            ),
            Tuple::TupleIndentEnd(cur) => self.stuck(
                "UNFINISHED PARENTHESES",
                start,
                *cur,
                reflow("I was expecting to see a closing parenthesis next:"),
                stack([try_adding(Bracket::Paren, '!'), closing_paren_note()]),
            ),
        }
    }

    /// `toListReport`.
    pub(super) fn list(&self, context: &Context<'_>, list: &List, start: Position) -> Problem {
        let trailing_comma = || {
            stack([
                reflow(
                    "Trailing commas are not allowed in lists, so the fix may be to delete the \
                     comma?",
                ),
                doc::simple_note(
                    "I recommend using the following format for lists that span multiple lines:",
                ),
                list_example(),
                convention('.'),
            ])
        };
        let indentation = || {
            doc::simple_note(
                "I may be confused by indentation. For example, if you are trying to define a \
                 list across multiple lines, I recommend using this format:",
            )
        };
        let end = || reflow("I cannot find the end of this list:");
        match list {
            List::ListSpace(space, cur) => self.space(*space, *cur),
            List::ListOpen(cur) | List::ListEnd(cur) => self.stuck(
                "UNFINISHED LIST",
                start,
                *cur,
                reflow("I am partway through parsing a list, but I got stuck here:"),
                closing_before_this(Bracket::Square),
            ),
            List::ListExpr(Expr::Start(c), _) => self.stuck(
                "UNFINISHED LIST",
                start,
                *c,
                reflow("I was expecting to see another list entry after that last comma:"),
                trailing_comma(),
            ),
            List::ListExpr(expr, cur) => self.expr(&context.node(Node::List, start), expr, *cur),
            List::ListIndentOpen(cur) => self.stuck(
                "UNFINISHED LIST",
                start,
                *cur,
                end(),
                stack([
                    fill![
                        "You could change it to something like",
                        dull_yellow("[3,4,5]"),
                        "or even just",
                        dull_yellow("[]").append(Doc::from(".")),
                        "Anything where there is an open and close square brace, and where the \
                         elements of the list are separated by commas."
                    ],
                    indentation(),
                    list_example(),
                    convention('.'),
                ]),
            ),
            List::ListIndentEnd(cur) => self.stuck(
                "UNFINISHED LIST",
                start,
                *cur,
                end(),
                stack([
                    fill![
                        "You can just add a closing",
                        dull_yellow("]"),
                        "right here, and I will be all set!"
                    ],
                    indentation(),
                    list_example(),
                    convention('.'),
                ]),
            ),
            List::ListIndentExpr(cur) => self.stuck(
                "UNFINISHED LIST",
                start,
                *cur,
                reflow("I was expecting to see another list entry after this comma:"),
                trailing_comma(),
            ),
        }
    }
}

/// What `RecordOpen` and `RecordField` expect: `a` field or `another`.
fn expecting_field(another: &str) -> Doc {
    stack([
        fill![
            "I was expecting to see",
            another,
            "record field defined next, so I am looking for a name like",
            dull_yellow("userName"),
            "or",
            dull_yellow("plantHeight").append(Doc::from("."))
        ],
        doc::simple_note(
            "Field names must start with a lower-case letter. After that, you can use any \
             sequence of letters, numbers, and underscores.",
        ),
        note_for_record_error(),
    ])
}

/// What `TupleOperatorReserved` suggests of each reserved symbol.
fn operator_function_hint(operator: BadOperator) -> Doc {
    match operator {
        BadOperator::BadDot => fill![
            "Maybe you wanted a record accessor like",
            dull_yellow(".x"),
            "or",
            dull_yellow(".name"),
            "instead?"
        ],
        BadOperator::BadPipe => fill![
            "Try",
            dull_yellow("(||)"),
            "instead? To turn boolean OR into a function?"
        ],
        BadOperator::BadArrow => fill![
            "Maybe you wanted",
            dull_yellow("(>)"),
            "or",
            dull_yellow("(>=)"),
            "instead?"
        ],
        BadOperator::BadEquals => fill![
            "Try",
            dull_yellow("(==)"),
            "instead? To make a function that checks equality?"
        ],
        BadOperator::BadHasType => fill![
            "Try",
            dull_yellow("(::)"),
            "instead? To add values to the front of lists?"
        ],
    }
}

/// `noteForRecordError` and `noteForRecordIndentError`, by their first
/// sentence and how their last ends.
fn record_note(intro: &str, ending: char) -> Doc {
    stack([
        doc::simple_note(intro),
        record_example(),
        convention(ending),
    ])
}

/// `noteForRecordError`.
fn note_for_record_error() -> Doc {
    record_note(
        "If you are trying to define a record across multiple lines, I recommend using this \
         format:",
        '.',
    )
}

/// `noteForRecordIndentError`.
fn note_for_record_indent_error() -> Doc {
    record_note(
        "I may be confused by indentation. For example, if you are trying to define a record \
         across multiple lines, I recommend using this format:",
        '!',
    )
}

fn record_example() -> Doc {
    vcat([
        Doc::from("{ name = ").append(dull_yellow("\"Alice\"")),
        Doc::from(", age = ").append(dull_yellow("42")),
        Doc::from(", height = ").append(dull_yellow("1.75")),
        Doc::from("}"),
    ])
    .indent(4)
}

fn list_example() -> Doc {
    vcat([
        Doc::from("[ ").append(dull_yellow("\"Alice\"")),
        Doc::from(", ").append(dull_yellow("\"Bob\"")),
        Doc::from(", ").append(dull_yellow("\"Chuck\"")),
        Doc::from("]"),
    ])
    .indent(4)
}
