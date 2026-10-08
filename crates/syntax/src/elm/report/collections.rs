//! The reports on expressions that collect others: `toRecordReport`,
//! `toTupleReport` and `toListReport`.

use nomi_base::diagnostic::code::{Next, Region};
use nomi_base::diagnostic::doc::{self, fill_sep, reflow, stack, vcat};
use nomi_base::diagnostic::{Doc, Problem};

use super::expr::{Context, Node};
use super::{Reporter, dullyellow, green, position, surroundings, to_region};
use crate::elm::error::{BadOperator, Cursor, Expr, List, Record, Tuple};

impl Reporter<'_, '_> {
    /// `toRecordReport`.
    pub(super) fn record(
        &self,
        context: &Context<'_>,
        record: &Record,
        start: Cursor,
    ) -> Option<Problem> {
        let stuck = |cur: Cursor, title: &'static str, before: &str, after: Doc| {
            self.within(
                title,
                surroundings(start, cur),
                to_region(cur),
                reflow(before),
                after,
            )
        };
        let reserved = |cur: Cursor, keyword: &str, before: &str| {
            self.within(
                "RESERVED WORD",
                surroundings(start, cur),
                self.keyword_region(cur, keyword),
                reflow(before),
                reflow(&format!(
                    "It looks like you are trying to use `{keyword}` as a field name, but that \
                     is a reserved word. Try using a different name!"
                )),
            )
        };
        let expecting_field = |another: &str| {
            stack([
                fill_sep(words![
                    "I was expecting to see",
                    another,
                    "record field defined next, so I am looking for a name like",
                    dullyellow("userName"),
                    "or",
                    dullyellow("plantHeight").append(Doc::from(".")),
                ]),
                doc::simple_note(
                    "Field names must start with a lower-case letter. After that, you can use \
                     any sequence of letters, numbers, and underscores.",
                ),
                note_for_record_error(),
            ])
        };
        let partway = "I am partway through parsing a record, but I got stuck here:";
        Some(match record {
            Record::RecordOpen(cur) => match self.next(*cur) {
                Next::Keyword(keyword) => reserved(
                    *cur,
                    keyword,
                    "I just started parsing a record, but I got stuck on this field name:",
                ),
                _ => stuck(
                    *cur,
                    "PROBLEM IN RECORD",
                    "I just started parsing a record, but I got stuck here:",
                    expecting_field("a"),
                ),
            },
            Record::RecordEnd(cur) => stuck(
                *cur,
                "PROBLEM IN RECORD",
                partway,
                stack([
                    fill_sep(words![
                        "I was expecting to see a closing curly brace before this, so try adding \
                         a",
                        dullyellow("}"),
                        "and see if that helps?",
                    ]),
                    doc::simple_note(
                        "When I get stuck like this, it usually means that there is a missing \
                         parenthesis or bracket somewhere earlier. It could also be a stray \
                         keyword or operator.",
                    ),
                ]),
            ),
            Record::RecordField(cur) => match self.next(*cur) {
                Next::Keyword(keyword) => reserved(
                    *cur,
                    keyword,
                    "I am partway through parsing a record, but I got stuck on this field name:",
                ),
                Next::Other(Some(',')) => stuck(
                    *cur,
                    "EXTRA COMMA",
                    partway,
                    stack([
                        reflow("I am seeing two commas in a row. This is the second one!"),
                        reflow("Just delete one of the commas and you should be all set!"),
                        note_for_record_error(),
                    ]),
                ),
                Next::Close(_, '}') => stuck(
                    *cur,
                    "EXTRA COMMA",
                    partway,
                    stack([
                        reflow(
                            "Trailing commas are not allowed in records. Try deleting the comma \
                             that appears before this closing curly brace.",
                        ),
                        note_for_record_error(),
                    ]),
                ),
                _ => stuck(
                    *cur,
                    "PROBLEM IN RECORD",
                    partway,
                    expecting_field("another"),
                ),
            },
            Record::RecordEquals(cur) => stuck(
                *cur,
                "PROBLEM IN RECORD",
                partway,
                stack([
                    fill_sep(words![
                        "I just saw a field name, so I was expecting to see an equals sign next. \
                         So try putting an",
                        green("="),
                        "sign here?",
                    ]),
                    note_for_record_error(),
                ]),
            ),
            Record::RecordExpr(expr, cur) => {
                return self.expr(&Context::Node(Node::Record, start, context), expr, *cur);
            }
            Record::RecordSpace(space, cur) => self.space(*space, *cur),
            Record::RecordIndentOpen(cur) => stuck(
                *cur,
                "UNFINISHED RECORD",
                "I just saw the opening curly brace of a record, but then I got stuck here:",
                stack([
                    fill_sep(words![
                        "I am expecting a record like",
                        dullyellow("{ x = 3, y = 4 }"),
                        "here. Try defining some fields of your own?",
                    ]),
                    note_for_record_indent_error(),
                ]),
            ),
            Record::RecordIndentEnd(cur) => {
                let partway = "I was partway through parsing a record, but I got stuck here:";
                match (self.source).next_line_starts_with_close_curly(position(*cur), self.compat) {
                    Some(curly) => self.within(
                        "NEED MORE INDENTATION",
                        Region {
                            start: position(start),
                            end: curly,
                        },
                        Region {
                            start: curly,
                            end: curly,
                        },
                        reflow(partway),
                        stack([
                            reflow(
                                "I need this curly brace to be indented more. Try adding some \
                                 spaces before it!",
                            ),
                            note_for_record_error(),
                        ]),
                    ),
                    None => stuck(
                        *cur,
                        "UNFINISHED RECORD",
                        partway,
                        stack([
                            fill_sep(words![
                                "I was expecting to see a closing curly brace next. Try putting a",
                                green("}"),
                                "next and see if that helps?",
                            ]),
                            note_for_record_indent_error(),
                        ]),
                    ),
                }
            }
            Record::RecordIndentField(cur) => stuck(
                *cur,
                "UNFINISHED RECORD",
                "I am partway through parsing a record, but I got stuck after that last comma:",
                stack([
                    reflow(
                        "Trailing commas are not allowed in records, so the fix may be to delete \
                         that last comma? Or maybe you were in the middle of defining an \
                         additional field?",
                    ),
                    note_for_record_error(),
                ]),
            ),
            Record::RecordIndentEquals(cur) => stuck(
                *cur,
                "UNFINISHED RECORD",
                "I am partway through parsing a record. I just saw a record field, so I was \
                 expecting to see an equals sign next:",
                stack([
                    fill_sep(words![
                        "Try putting an",
                        green("="),
                        "followed by an expression?",
                    ]),
                    note_for_record_indent_error(),
                ]),
            ),
            Record::RecordIndentExpr(cur) => stuck(
                *cur,
                "UNFINISHED RECORD",
                "I am partway through parsing a record, and I was expecting to run into an \
                 expression next:",
                stack([
                    fill_sep(words![
                        "Try putting something like",
                        dullyellow("42"),
                        "or",
                        dullyellow("\"hello\""),
                        "for now?",
                    ]),
                    note_for_record_indent_error(),
                ]),
            ),
        })
    }

    /// `toTupleReport`.
    pub(super) fn tuple(
        &self,
        context: &Context<'_>,
        tuple: &Tuple,
        start: Cursor,
    ) -> Option<Problem> {
        let stuck = |cur: Cursor, title: &'static str, before: &str, after: Doc| {
            self.within(
                title,
                surroundings(start, cur),
                to_region(cur),
                reflow(before),
                after,
            )
        };
        let try_adding = |punctuation: &str| {
            fill_sep(words![
                "Try adding a",
                dullyellow(")"),
                "to see if that",
                Doc::from(format!("helps{punctuation}")),
            ])
        };
        let not_indented = || {
            doc::simple_note(
                "I can get confused by indentation in cases like this, so maybe you have an \
                 expression but it is not indented enough?",
            )
        };
        Some(match tuple {
            Tuple::TupleExpr(expr, cur) => {
                return self.expr(&Context::Node(Node::Parens, start, context), expr, *cur);
            }
            Tuple::TupleSpace(space, cur) => self.space(*space, *cur),
            Tuple::TupleEnd(cur) => stuck(
                *cur,
                "UNFINISHED PARENTHESES",
                "I was expecting to see a closing parentheses next, but I got stuck here:",
                stack([
                    try_adding("?"),
                    doc::simple_note(
                        "I can get stuck when I run into keywords, operators, parentheses, or \
                         brackets unexpectedly. So there may be some earlier syntax trouble \
                         (like extra parenthesis or missing brackets) that is confusing me.",
                    ),
                ]),
            ),
            Tuple::TupleOperatorClose(cur) => stuck(
                *cur,
                "UNFINISHED OPERATOR FUNCTION",
                "I was expecting a closing parenthesis here:",
                stack([
                    try_adding("!"),
                    doc::simple_note(
                        "I think I am parsing an operator function right now, so I am expecting \
                         to see something like (+) or (&&) where an operator is surrounded by \
                         parentheses with no extra spaces.",
                    ),
                ]),
            ),
            Tuple::TupleOperatorReserved(operator, cur) => stuck(
                *cur,
                "UNEXPECTED SYMBOL",
                "I ran into an unexpected symbol here:",
                match operator {
                    BadOperator::BadDot => fill_sep(words![
                        "Maybe you wanted a record accessor like",
                        dullyellow(".x"),
                        "or",
                        dullyellow(".name"),
                        "instead?",
                    ]),
                    BadOperator::BadPipe => fill_sep(words![
                        "Try",
                        dullyellow("(||)"),
                        "instead? To turn boolean OR into a function?",
                    ]),
                    BadOperator::BadArrow => fill_sep(words![
                        "Maybe you wanted",
                        dullyellow("(>)"),
                        "or",
                        dullyellow("(>=)"),
                        "instead?",
                    ]),
                    BadOperator::BadEquals => fill_sep(words![
                        "Try",
                        dullyellow("(==)"),
                        "instead? To make a function that checks equality?",
                    ]),
                    BadOperator::BadHasType => fill_sep(words![
                        "Try",
                        dullyellow("(::)"),
                        "instead? To add values to the front of lists?",
                    ]),
                },
            ),
            Tuple::TupleIndentExpr1(cur) => stuck(
                *cur,
                "UNFINISHED PARENTHESES",
                "I just saw an open parenthesis, so I was expecting to see an expression next.",
                stack([
                    fill_sep(words![
                        "Something like",
                        dullyellow("(4 + 5)"),
                        "or",
                        dullyellow("(String.reverse \"desserts\")").append(Doc::from(".")),
                        "Anything where you are putting parentheses around normal expressions.",
                    ]),
                    not_indented(),
                ]),
            ),
            Tuple::TupleIndentExprN(cur) => stuck(
                *cur,
                "UNFINISHED TUPLE",
                "I think I am in the middle of parsing a tuple. I just saw a comma, so I was \
                 expecting to see an expression next.",
                stack([
                    fill_sep(words![
                        "A tuple looks like",
                        dullyellow("(3,4)"),
                        "or",
                        dullyellow("(\"Tom\",42)").append(Doc::from(",")),
                        "so I think there is an expression missing here?",
                    ]),
                    not_indented(),
                ]),
            ),
            Tuple::TupleIndentEnd(cur) => stuck(
                *cur,
                "UNFINISHED PARENTHESES",
                "I was expecting to see a closing parenthesis next:",
                stack([
                    try_adding("!"),
                    doc::simple_note(
                        "I can get confused by indentation in cases like this, so maybe you have \
                         a closing parenthesis but it is not indented enough?",
                    ),
                ]),
            ),
        })
    }

    /// `toListReport`.
    pub(super) fn list(
        &self,
        context: &Context<'_>,
        list: &List,
        start: Cursor,
    ) -> Option<Problem> {
        let stuck = |cur: Cursor, before: &str, after: Doc| {
            self.within(
                "UNFINISHED LIST",
                surroundings(start, cur),
                to_region(cur),
                reflow(before),
                after,
            )
        };
        let closing = || {
            stack([
                fill_sep(words![
                    "I was expecting to see a closing square bracket before this, so try adding a",
                    dullyellow("]"),
                    "and see if that helps?",
                ]),
                doc::simple_note(
                    "When I get stuck like this, it usually means that there is a missing \
                     parenthesis or bracket somewhere earlier. It could also be a stray keyword \
                     or operator.",
                ),
            ])
        };
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
                list_convention(),
            ])
        };
        let indentation = || {
            doc::simple_note(
                "I may be confused by indentation. For example, if you are trying to define a \
                 list across multiple lines, I recommend using this format:",
            )
        };
        let partway = "I am partway through parsing a list, but I got stuck here:";
        Some(match list {
            List::ListSpace(space, cur) => self.space(*space, *cur),
            List::ListOpen(cur) | List::ListEnd(cur) => stuck(*cur, partway, closing()),
            List::ListExpr(Expr::Start(c), _) => stuck(
                *c,
                "I was expecting to see another list entry after that last comma:",
                trailing_comma(),
            ),
            List::ListExpr(expr, cur) => {
                return self.expr(&Context::Node(Node::List, start, context), expr, *cur);
            }
            List::ListIndentOpen(cur) => stuck(
                *cur,
                "I cannot find the end of this list:",
                stack([
                    fill_sep(words![
                        "You could change it to something like",
                        dullyellow("[3,4,5]"),
                        "or even just",
                        dullyellow("[]").append(Doc::from(".")),
                        "Anything where there is an open and close square brace, and where the \
                         elements of the list are separated by commas.",
                    ]),
                    indentation(),
                    list_example(),
                    list_convention(),
                ]),
            ),
            List::ListIndentEnd(cur) => stuck(
                *cur,
                "I cannot find the end of this list:",
                stack([
                    fill_sep(words![
                        "You can just add a closing",
                        dullyellow("]"),
                        "right here, and I will be all set!",
                    ]),
                    indentation(),
                    list_example(),
                    list_convention(),
                ]),
            ),
            List::ListIndentExpr(cur) => stuck(
                *cur,
                "I was expecting to see another list entry after this comma:",
                trailing_comma(),
            ),
        })
    }
}

/// `noteForRecordError`.
fn note_for_record_error() -> Doc {
    stack([
        doc::simple_note(
            "If you are trying to define a record across multiple lines, I recommend using this \
             format:",
        ),
        record_example(),
        reflow(
            "Notice that each line starts with some indentation. Usually two or four spaces. This \
             is the stylistic convention in the Elm ecosystem.",
        ),
    ])
}

/// `noteForRecordIndentError`.
fn note_for_record_indent_error() -> Doc {
    stack([
        doc::simple_note(
            "I may be confused by indentation. For example, if you are trying to define a record \
             across multiple lines, I recommend using this format:",
        ),
        record_example(),
        reflow(
            "Notice that each line starts with some indentation. Usually two or four spaces. This \
             is the stylistic convention in the Elm ecosystem!",
        ),
    ])
}

fn record_example() -> Doc {
    vcat([
        Doc::from("{ name = ").append(dullyellow("\"Alice\"")),
        Doc::from(", age = ").append(dullyellow("42")),
        Doc::from(", height = ").append(dullyellow("1.75")),
        Doc::from("}"),
    ])
    .indent(4)
}

fn list_example() -> Doc {
    vcat([
        Doc::from("[ ").append(dullyellow("\"Alice\"")),
        Doc::from(", ").append(dullyellow("\"Bob\"")),
        Doc::from(", ").append(dullyellow("\"Chuck\"")),
        Doc::from("]"),
    ])
    .indent(4)
}

fn list_convention() -> Doc {
    reflow(
        "Notice that each line starts with some indentation. Usually two or four spaces. This is \
         the stylistic convention in the Elm ecosystem.",
    )
}
