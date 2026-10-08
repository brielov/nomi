//! The reports on patterns: `toPatternReport` with the records, tuples
//! and lists it leads to.

use nomi_base::diagnostic::code::Next;
use nomi_base::diagnostic::doc::{self, fill_sep, reflow, stack};
use nomi_base::diagnostic::{Doc, Problem};

use super::{Reporter, cyan, dullyellow, surroundings, to_region};
use crate::elm::error::{Cursor, PList, PRecord, PTuple, Pattern};

/// `PContext`: where a pattern is.
#[derive(Clone, Copy)]
pub(super) enum PContext {
    Case,
    Arg,
    Let,
}

impl Reporter<'_, '_> {
    /// `toPatternReport`.
    pub(super) fn pattern(&self, context: PContext, pattern: &Pattern, start: Cursor) -> Problem {
        let stuck = || {
            fill_sep(words![
                "I am not sure why I am getting stuck exactly. I just know that I want a pattern \
                 next. Something as simple as",
                dullyellow("maybeHeight"),
                "or",
                dullyellow("result"),
                "would work!",
            ])
        };
        let alias = |cur: Cursor, point: Doc| {
            self.at(
                "UNFINISHED PATTERN",
                to_region(cur),
                reflow("I was expecting to see a variable name after the `as` keyword:"),
                stack([
                    fill_sep(words![
                        "The `as` keyword lets you write patterns like",
                        Doc::from("((")
                            .append(dullyellow("x"))
                            .append(Doc::from(","))
                            .append(dullyellow("y"))
                            .append(Doc::from(") "))
                            .append(cyan("as"))
                            .append(dullyellow(" point"))
                            .append(Doc::from(")")),
                        "so you can refer to individual parts of the tuple with",
                        dullyellow("x"),
                        "and",
                        dullyellow("y"),
                        "or you refer to the whole thing with",
                        point,
                    ]),
                    reflow(
                        "So I was expecting to see a variable name after the `as` keyword here. \
                         Sometimes people just want to use `as` as a variable name though. Try \
                         using a different name in that case!",
                    ),
                ]),
            )
        };
        match pattern {
            Pattern::PRecord(record, cur) => self.precord(record, *cur),
            Pattern::PTuple(tuple, cur) => self.ptuple(context, tuple, *cur),
            Pattern::PList(list, cur) => self.plist(context, list, *cur),
            Pattern::PStart(cur) => match self.next(*cur) {
                Next::Keyword(keyword) => self.within(
                    "RESERVED WORD",
                    surroundings(start, *cur),
                    self.keyword_region(*cur, keyword),
                    reflow(&format!(
                        "It looks like you are trying to use `{keyword}` {}:",
                        match context {
                            PContext::Arg => "as an argument",
                            PContext::Case | PContext::Let => "in this pattern",
                        }
                    )),
                    reflow("This is a reserved word! Try using some other name?"),
                ),
                Next::Operator(op) if op == "-" => self.within(
                    "UNEXPECTED SYMBOL",
                    surroundings(start, *cur),
                    to_region(*cur),
                    reflow("I ran into a minus sign unexpectedly in this pattern:"),
                    reflow(
                        "It is not possible to pattern match on negative numbers at this time. \
                         Try using an `if` expression for that sort of thing for now.",
                    ),
                ),
                _ => self.within(
                    "PROBLEM IN PATTERN",
                    surroundings(start, *cur),
                    to_region(*cur),
                    reflow("I wanted to parse a pattern next, but I got stuck here:"),
                    stuck(),
                ),
            },
            Pattern::PChar(char, cur) => self.char(char, *cur),
            Pattern::PString(string, cur) => self.string(string, *cur),
            Pattern::PNumber(number, cur) => self.number(number, *cur),
            Pattern::PFloat(width, cur) => self.at(
                "UNEXPECTED PATTERN",
                self.wider(*cur, *width as usize),
                reflow("I cannot pattern match with floating point numbers:"),
                fill_sep(words![
                    "Equality on floats can be unreliable, so you usually want to check that \
                     they are nearby with some sort of",
                    dullyellow("(abs (actual - expected) < 0.001)"),
                    "check.",
                ]),
            ),
            Pattern::PAlias(cur) => alias(*cur, dullyellow("point").append(Doc::from("."))),
            Pattern::PWildcardNotVar(name, width, cur) => {
                let examples: Vec<Doc> = match name.trim_start_matches('_') {
                    "" => vec![dullyellow("x"), Doc::from("or"), dullyellow("age")],
                    rest => {
                        let mut chars = rest.chars();
                        let first = chars.next().into_iter().flat_map(char::to_lowercase);
                        vec![dullyellow(first.chain(chars).collect::<String>())]
                    }
                };
                self.at(
                    "UNEXPECTED NAME",
                    self.wider(*cur, *width as usize),
                    reflow("Variable names cannot start with underscores like this:"),
                    fill_sep(
                        words![
                            "You can either have an underscore like",
                            dullyellow("_"),
                            "to ignore the value, or you can have a name like",
                        ]
                        .chain(examples)
                        .chain(words!["to use the matched value."]),
                    ),
                )
            }
            Pattern::PSpace(space, cur) => self.space(*space, *cur),
            Pattern::PIndentStart(cur) => self.within(
                "UNFINISHED PATTERN",
                surroundings(start, *cur),
                to_region(*cur),
                reflow("I wanted to parse a pattern next, but I got stuck here:"),
                stack([
                    stuck(),
                    doc::simple_note(
                        "I can get confused by indentation. If you think there is a pattern \
                         next, maybe it needs to be indented a bit more?",
                    ),
                ]),
            ),
            Pattern::PIndentAlias(cur) => alias(*cur, dullyellow("point.")),
        }
    }

    /// `toPRecordReport`.
    fn precord(&self, record: &PRecord, start: Cursor) -> Problem {
        let field_name = || reflow("I was expecting to see a field name next.");
        let closing = || {
            fill_sep(words![
                "I was expecting to see a closing curly brace next. Try adding a",
                dullyellow("}"),
                "here?",
            ])
        };
        match record {
            PRecord::PRecordOpen(cur)
            | PRecord::PRecordIndentOpen(cur)
            | PRecord::PRecordIndentField(cur) => {
                self.unfinished_record_pattern(*cur, start, field_name())
            }
            PRecord::PRecordEnd(cur) | PRecord::PRecordIndentEnd(cur) => {
                self.unfinished_record_pattern(*cur, start, closing())
            }
            PRecord::PRecordField(cur) => match self.next(*cur) {
                Next::Keyword(keyword) => self.within(
                    "RESERVED WORD",
                    surroundings(start, *cur),
                    self.keyword_region(*cur, keyword),
                    reflow(&format!(
                        "I was not expecting to see `{keyword}` as a record field name:"
                    )),
                    reflow(
                        "This is a reserved word, not available for variable names. Try another \
                         name!",
                    ),
                ),
                _ => self.unfinished_record_pattern(*cur, start, field_name()),
            },
            PRecord::PRecordSpace(space, cur) => self.space(*space, *cur),
        }
    }

    /// `toUnfinishRecordPatternReport`.
    fn unfinished_record_pattern(&self, cur: Cursor, start: Cursor, message: Doc) -> Problem {
        self.within(
            "UNFINISHED RECORD PATTERN",
            surroundings(start, cur),
            to_region(cur),
            reflow("I was partway through parsing a record pattern, but I got stuck here:"),
            stack([
                message,
                doc::hint(words![
                    "A record pattern looks like",
                    dullyellow("{x,y}"),
                    "or",
                    dullyellow("{name,age}"),
                    "where you list the field names you want to access.",
                ]),
            ]),
        )
    }

    /// `toPTupleReport`.
    fn ptuple(&self, context: PContext, tuple: &PTuple, start: Cursor) -> Problem {
        let stuck = |cur: Cursor, title: &'static str, before: &str, after: Doc| {
            self.within(
                title,
                surroundings(start, cur),
                to_region(cur),
                reflow(before),
                after,
            )
        };
        let expecting_pattern = || {
            fill_sep(words![
                "I was expecting to see a pattern next. Maybe it will end up being something like",
                dullyellow("(x,y)"),
                "or",
                dullyellow("(name, _)").append(Doc::from("?")),
            ])
        };
        let try_adding = || {
            fill_sep(words![
                "Try adding a",
                dullyellow(")"),
                "to see if that helps?",
            ])
        };
        match tuple {
            PTuple::PTupleOpen(cur) => match self.next(*cur) {
                Next::Keyword(keyword) => self.within(
                    "RESERVED WORD",
                    surroundings(start, *cur),
                    self.keyword_region(*cur, keyword),
                    reflow(&format!(
                        "It looks like you are trying to use `{keyword}` as a variable name:"
                    )),
                    reflow("This is a reserved word! Try using some other name?"),
                ),
                _ => stuck(
                    *cur,
                    "UNFINISHED PARENTHESES",
                    "I just saw an open parenthesis, but I got stuck here:",
                    expecting_pattern(),
                ),
            },
            PTuple::PTupleEnd(cur) => match self.next(*cur) {
                Next::Keyword(keyword) => self.within(
                    "RESERVED WORD",
                    surroundings(start, *cur),
                    self.keyword_region(*cur, keyword),
                    reflow("I ran into a reserved word in this pattern:"),
                    reflow(&format!(
                        "The `{keyword}` keyword is reserved. Try using a different name instead!"
                    )),
                ),
                Next::Operator(op) => self.within(
                    "UNEXPECTED SYMBOL",
                    surroundings(start, *cur),
                    self.keyword_region(*cur, &op),
                    reflow(&format!(
                        "I ran into the {op} symbol unexpectedly in this pattern:"
                    )),
                    reflow(
                        "Only the :: symbol that works in patterns. It is useful if you are \
                         pattern matching on lists, trying to get the first element off the \
                         front. Did you want that instead?",
                    ),
                ),
                Next::Close(term, bracket) => self.within(
                    format!("STRAY {}", term.to_uppercase()),
                    surroundings(start, *cur),
                    to_region(*cur),
                    reflow(&format!(
                        "I ran into a an unexpected {term} in this pattern:"
                    )),
                    reflow(&format!(
                        "This {bracket} does not match up with an earlier open {term}. Try \
                         deleting it?"
                    )),
                ),
                _ => stuck(
                    *cur,
                    "UNFINISHED PARENTHESES",
                    "I was partway through parsing a pattern, but I got stuck here:",
                    fill_sep(words![
                        "I was expecting a closing parenthesis next, so try adding a",
                        dullyellow(")"),
                        "to see if that helps?",
                    ]),
                ),
            },
            PTuple::PTupleExpr(pattern, cur) => self.pattern(context, pattern, *cur),
            PTuple::PTupleSpace(space, cur) => self.space(*space, *cur),
            PTuple::PTupleIndentEnd(cur) => stuck(
                *cur,
                "UNFINISHED PARENTHESES",
                "I was expecting a closing parenthesis next:",
                stack([
                    try_adding(),
                    doc::simple_note(
                        "I can get confused by indentation in cases like this, so maybe you have \
                         a closing parenthesis but it is not indented enough?",
                    ),
                ]),
            ),
            PTuple::PTupleIndentExpr1(cur) => stuck(
                *cur,
                "UNFINISHED PARENTHESES",
                "I just saw an open parenthesis, but then I got stuck here:",
                expecting_pattern(),
            ),
            PTuple::PTupleIndentExprN(cur) => stuck(
                *cur,
                "UNFINISHED TUPLE PATTERN",
                "I am partway through parsing a tuple pattern, but I got stuck here:",
                stack([
                    fill_sep(words![
                        "I was expecting to see a pattern next. I am expecting the final result \
                         to be something like",
                        dullyellow("(x,y)"),
                        "or",
                        dullyellow("(name, _)").append(Doc::from(".")),
                    ]),
                    doc::simple_note(
                        "I can get confused by indentation in cases like this, so the problem \
                         may be that the next part is not indented enough?",
                    ),
                ]),
            ),
        }
    }

    /// `toPListReport`.
    fn plist(&self, context: PContext, list: &PList, start: Cursor) -> Problem {
        let stuck = |cur: Cursor, before: &str, after: Doc| {
            self.within(
                "UNFINISHED LIST PATTERN",
                surroundings(start, cur),
                to_region(cur),
                reflow(before),
                after,
            )
        };
        let try_adding = || {
            fill_sep(words![
                "Try adding a",
                dullyellow("]"),
                "to see if that helps?",
            ])
        };
        let open = "I just saw an open square bracket, but then I got stuck here:";
        let end = "I was expecting a closing square bracket to end this list pattern:";
        match list {
            PList::PListOpen(cur) => match self.next(*cur) {
                Next::Keyword(keyword) => self.within(
                    "RESERVED WORD",
                    surroundings(start, *cur),
                    self.keyword_region(*cur, keyword),
                    reflow(&format!(
                        "It looks like you are trying to use `{keyword}` to name an element of a \
                         list:"
                    )),
                    reflow("This is a reserved word though! Try using some other name?"),
                ),
                _ => stuck(*cur, open, try_adding()),
            },
            PList::PListEnd(cur) => stuck(*cur, end, try_adding()),
            PList::PListExpr(pattern, cur) => self.pattern(context, pattern, *cur),
            PList::PListSpace(space, cur) => self.space(*space, *cur),
            PList::PListIndentOpen(cur) => stuck(
                *cur,
                open,
                stack([
                    try_adding(),
                    doc::simple_note(
                        "I can get confused by indentation in cases like this, so maybe there is \
                         something next, but it is not indented enough?",
                    ),
                ]),
            ),
            PList::PListIndentEnd(cur) => stuck(
                *cur,
                end,
                stack([
                    try_adding(),
                    doc::simple_note(
                        "I can get confused by indentation in cases like this, so maybe you have \
                         a closing square bracket but it is not indented enough?",
                    ),
                ]),
            ),
            PList::PListIndentExpr(cur) => stuck(
                *cur,
                "I am partway through parsing a list pattern, but I got stuck here:",
                stack([
                    reflow("I was expecting to see another pattern next. Maybe a variable name."),
                    doc::simple_note(
                        "I can get confused by indentation in cases like this, so maybe there is \
                         more to this pattern but it is not indented enough?",
                    ),
                ]),
            ),
        }
    }
}
