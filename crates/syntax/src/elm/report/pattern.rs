//! The reports on patterns: `toPatternReport` with the records, tuples
//! and lists it leads to.

use nomi_base::diagnostic::code::{Bracket, Next};
use nomi_base::diagnostic::doc::{self, cyan, dull_yellow, reflow, stack};
use nomi_base::diagnostic::{Doc, Problem};

use super::{
    Reporter, closing_paren_note, stray_title, surroundings, to_region, try_adding, unmatched_close,
};
use crate::elm::error::{Name, PList, PRecord, PTuple, Pattern, Position};

/// `PContext`: where a pattern is.
#[derive(Clone, Copy)]
pub(super) enum PContext {
    Case,
    Arg,
    Let,
}

impl Reporter<'_, '_> {
    /// `toPatternReport`.
    pub(super) fn pattern(&self, context: PContext, pattern: &Pattern, start: Position) -> Problem {
        match pattern {
            Pattern::PRecord(record, cur) => self.precord(record, *cur),
            Pattern::PTuple(tuple, cur) => self.ptuple(context, tuple, *cur),
            Pattern::PList(list, cur) => self.plist(context, list, *cur),
            Pattern::PStart(cur) => self.pattern_start(context, start, *cur),
            Pattern::PChar(char, cur) => self.char(char, *cur),
            Pattern::PString(string, cur) => self.string(string, *cur),
            Pattern::PNumber(number, cur) => self.number(number, *cur),
            Pattern::PFloat(width, cur) => self.at(
                "UNEXPECTED PATTERN",
                self.wider(*cur, *width),
                reflow("I cannot pattern match with floating point numbers:"),
                fill![
                    "Equality on floats can be unreliable, so you usually want to check that they \
                     are nearby with some sort of",
                    dull_yellow("(abs (actual - expected) < 0.001)"),
                    "check."
                ],
            ),
            Pattern::PAlias(cur) => self.alias(*cur, dull_yellow("point").append(Doc::from("."))),
            Pattern::PWildcardNotVar { name, width, at } => {
                self.wildcard_not_var(name, *width, *at)
            }
            Pattern::PSpace(space, cur) => self.space(*space, *cur),
            Pattern::PIndentStart(cur) => self.stuck(
                "UNFINISHED PATTERN",
                start,
                *cur,
                reflow("I wanted to parse a pattern next, but I got stuck here:"),
                stack([
                    want_pattern(),
                    doc::simple_note(
                        "I can get confused by indentation. If you think there is a pattern \
                         next, maybe it needs to be indented a bit more?",
                    ),
                ]),
            ),
            Pattern::PIndentAlias(cur) => self.alias(*cur, dull_yellow("point.")),
        }
    }

    /// `PStart`, by what is there.
    fn pattern_start(&self, context: PContext, start: Position, cur: Position) -> Problem {
        match self.next(cur) {
            Next::Keyword(keyword) => self.reserved(
                start,
                cur,
                keyword,
                reflow(&format!(
                    "It looks like you are trying to use `{keyword}` {}:",
                    match context {
                        PContext::Arg => "as an argument",
                        PContext::Case | PContext::Let => "in this pattern",
                    }
                )),
                reflow("This is a reserved word! Try using some other name?"),
            ),
            Next::Operator(op) if op == "-" => self.stuck(
                "UNEXPECTED SYMBOL",
                start,
                cur,
                reflow("I ran into a minus sign unexpectedly in this pattern:"),
                reflow(
                    "It is not possible to pattern match on negative numbers at this time. Try \
                     using an `if` expression for that sort of thing for now.",
                ),
            ),
            Next::Operator(_)
            | Next::Close(_)
            | Next::Upper(..)
            | Next::Lower(..)
            | Next::Other(_) => self.stuck(
                "PROBLEM IN PATTERN",
                start,
                cur,
                reflow("I wanted to parse a pattern next, but I got stuck here:"),
                want_pattern(),
            ),
        }
    }

    /// `PAlias` and `PIndentAlias`, which end their hint with `point`.
    fn alias(&self, cur: Position, point: Doc) -> Problem {
        self.at(
            "UNFINISHED PATTERN",
            to_region(cur),
            reflow("I was expecting to see a variable name after the `as` keyword:"),
            stack([
                fill![
                    "The `as` keyword lets you write patterns like",
                    Doc::from("((")
                        .append(dull_yellow("x"))
                        .append(Doc::from(","))
                        .append(dull_yellow("y"))
                        .append(Doc::from(") "))
                        .append(cyan("as"))
                        .append(dull_yellow(" point"))
                        .append(Doc::from(")")),
                    "so you can refer to individual parts of the tuple with",
                    dull_yellow("x"),
                    "and",
                    dull_yellow("y"),
                    "or you refer to the whole thing with",
                    point
                ],
                reflow(
                    "So I was expecting to see a variable name after the `as` keyword here. \
                     Sometimes people just want to use `as` as a variable name though. Try using \
                     a different name in that case!",
                ),
            ]),
        )
    }

    /// `PWildcardNotVar`: `name`, `width` wide, starts with underscores.
    fn wildcard_not_var(&self, name: &Name, width: u32, at: Position) -> Problem {
        let examples: Vec<Doc> = match name.trim_start_matches('_') {
            "" => vec![dull_yellow("x"), Doc::from("or"), dull_yellow("age")],
            rest => {
                let mut chars = rest.chars();
                let first = chars.next().into_iter().flat_map(char::to_lowercase);
                vec![dull_yellow(first.chain(chars).collect::<String>())]
            }
        };
        self.at(
            "UNEXPECTED NAME",
            self.wider(at, width),
            reflow("Variable names cannot start with underscores like this:"),
            doc::fill_sep(
                words![
                    "You can either have an underscore like",
                    dull_yellow("_"),
                    "to ignore the value, or you can have a name like"
                ]
                .chain(examples)
                .chain(words!["to use the matched value."]),
            ),
        )
    }

    /// `toPRecordReport`.
    fn precord(&self, record: &PRecord, start: Position) -> Problem {
        let field_name = || reflow("I was expecting to see a field name next.");
        match record {
            PRecord::PRecordOpen(cur)
            | PRecord::PRecordIndentOpen(cur)
            | PRecord::PRecordIndentField(cur) => {
                self.unfinished_record_pattern(*cur, start, field_name())
            }
            PRecord::PRecordEnd(cur) | PRecord::PRecordIndentEnd(cur) => self
                .unfinished_record_pattern(
                    *cur,
                    start,
                    fill![
                        "I was expecting to see a closing curly brace next. Try adding a",
                        dull_yellow("}"),
                        "here?"
                    ],
                ),
            PRecord::PRecordField(cur) => match self.next_keyword(*cur) {
                Some(keyword) => self.reserved(
                    start,
                    *cur,
                    keyword,
                    reflow(&format!(
                        "I was not expecting to see `{keyword}` as a record field name:"
                    )),
                    reflow(
                        "This is a reserved word, not available for variable names. Try another \
                         name!",
                    ),
                ),
                None => self.unfinished_record_pattern(*cur, start, field_name()),
            },
            PRecord::PRecordSpace(space, cur) => self.space(*space, *cur),
        }
    }

    /// `toUnfinishRecordPatternReport`.
    fn unfinished_record_pattern(&self, cur: Position, start: Position, message: Doc) -> Problem {
        self.stuck(
            "UNFINISHED RECORD PATTERN",
            start,
            cur,
            reflow("I was partway through parsing a record pattern, but I got stuck here:"),
            stack([
                message,
                doc::hint(words![
                    "A record pattern looks like",
                    dull_yellow("{x,y}"),
                    "or",
                    dull_yellow("{name,age}"),
                    "where you list the field names you want to access."
                ]),
            ]),
        )
    }

    /// `toPTupleReport`.
    fn ptuple(&self, context: PContext, tuple: &PTuple, start: Position) -> Problem {
        let expecting_pattern = || {
            fill![
                "I was expecting to see a pattern next. Maybe it will end up being something like",
                dull_yellow("(x,y)"),
                "or",
                dull_yellow("(name, _)").append(Doc::from("?"))
            ]
        };
        match tuple {
            PTuple::PTupleOpen(cur) => match self.next_keyword(*cur) {
                Some(keyword) => self.reserved(
                    start,
                    *cur,
                    keyword,
                    reflow(&format!(
                        "It looks like you are trying to use `{keyword}` as a variable name:"
                    )),
                    reflow("This is a reserved word! Try using some other name?"),
                ),
                None => self.stuck(
                    "UNFINISHED PARENTHESES",
                    start,
                    *cur,
                    reflow("I just saw an open parenthesis, but I got stuck here:"),
                    expecting_pattern(),
                ),
            },
            PTuple::PTupleEnd(cur) => self.ptuple_end(start, *cur),
            PTuple::PTupleExpr(pattern, cur) => self.pattern(context, pattern, *cur),
            PTuple::PTupleSpace(space, cur) => self.space(*space, *cur),
            PTuple::PTupleIndentEnd(cur) => self.stuck(
                "UNFINISHED PARENTHESES",
                start,
                *cur,
                reflow("I was expecting a closing parenthesis next:"),
                stack([try_adding(Bracket::Paren, '?'), closing_paren_note()]),
            ),
            PTuple::PTupleIndentExpr1(cur) => self.stuck(
                "UNFINISHED PARENTHESES",
                start,
                *cur,
                reflow("I just saw an open parenthesis, but then I got stuck here:"),
                expecting_pattern(),
            ),
            PTuple::PTupleIndentExprN(cur) => self.stuck(
                "UNFINISHED TUPLE PATTERN",
                start,
                *cur,
                reflow("I am partway through parsing a tuple pattern, but I got stuck here:"),
                stack([
                    fill![
                        "I was expecting to see a pattern next. I am expecting the final result to \
                         be something like",
                        dull_yellow("(x,y)"),
                        "or",
                        dull_yellow("(name, _)").append(Doc::from("."))
                    ],
                    doc::simple_note(
                        "I can get confused by indentation in cases like this, so the problem \
                         may be that the next part is not indented enough?",
                    ),
                ]),
            ),
        }
    }

    /// `PTupleEnd`, by what is there.
    fn ptuple_end(&self, start: Position, cur: Position) -> Problem {
        match self.next(cur) {
            Next::Keyword(keyword) => self.reserved(
                start,
                cur,
                keyword,
                reflow("I ran into a reserved word in this pattern:"),
                reflow(&format!(
                    "The `{keyword}` keyword is reserved. Try using a different name instead!"
                )),
            ),
            Next::Operator(op) => self.within(
                "UNEXPECTED SYMBOL",
                surroundings(start, cur),
                self.keyword_region(cur, &op),
                reflow(&format!(
                    "I ran into the {op} symbol unexpectedly in this pattern:"
                )),
                reflow(
                    "Only the :: symbol that works in patterns. It is useful if you are pattern \
                     matching on lists, trying to get the first element off the front. Did you \
                     want that instead?",
                ),
            ),
            Next::Close(close) => self.stuck(
                stray_title(close),
                start,
                cur,
                reflow(&format!(
                    "I ran into a an unexpected {} in this pattern:",
                    close.name()
                )),
                unmatched_close(close),
            ),
            Next::Upper(..) | Next::Lower(..) | Next::Other(_) => self.stuck(
                "UNFINISHED PARENTHESES",
                start,
                cur,
                reflow("I was partway through parsing a pattern, but I got stuck here:"),
                fill![
                    "I was expecting a closing parenthesis next, so try adding a",
                    dull_yellow(")"),
                    "to see if that helps?"
                ],
            ),
        }
    }

    /// `toPListReport`.
    fn plist(&self, context: PContext, list: &PList, start: Position) -> Problem {
        let open = || reflow("I just saw an open square bracket, but then I got stuck here:");
        let end = || reflow("I was expecting a closing square bracket to end this list pattern:");
        let unfinished = "UNFINISHED LIST PATTERN";
        match list {
            PList::PListOpen(cur) => match self.next_keyword(*cur) {
                Some(keyword) => self.reserved(
                    start,
                    *cur,
                    keyword,
                    reflow(&format!(
                        "It looks like you are trying to use `{keyword}` to name an element of a \
                         list:"
                    )),
                    reflow("This is a reserved word though! Try using some other name?"),
                ),
                None => self.stuck(
                    unfinished,
                    start,
                    *cur,
                    open(),
                    try_adding(Bracket::Square, '?'),
                ),
            },
            PList::PListEnd(cur) => self.stuck(
                unfinished,
                start,
                *cur,
                end(),
                try_adding(Bracket::Square, '?'),
            ),
            PList::PListExpr(pattern, cur) => self.pattern(context, pattern, *cur),
            PList::PListSpace(space, cur) => self.space(*space, *cur),
            PList::PListIndentOpen(cur) => self.stuck(
                unfinished,
                start,
                *cur,
                open(),
                stack([
                    try_adding(Bracket::Square, '?'),
                    doc::simple_note(
                        "I can get confused by indentation in cases like this, so maybe there is \
                         something next, but it is not indented enough?",
                    ),
                ]),
            ),
            PList::PListIndentEnd(cur) => self.stuck(
                unfinished,
                start,
                *cur,
                end(),
                stack([
                    try_adding(Bracket::Square, '?'),
                    doc::simple_note(
                        "I can get confused by indentation in cases like this, so maybe you have \
                         a closing square bracket but it is not indented enough?",
                    ),
                ]),
            ),
            PList::PListIndentExpr(cur) => self.stuck(
                unfinished,
                start,
                *cur,
                reflow("I am partway through parsing a list pattern, but I got stuck here:"),
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

/// What a pattern's report says when it can't say more.
fn want_pattern() -> Doc {
    fill![
        "I am not sure why I am getting stuck exactly. I just know that I want a pattern next. \
         Something as simple as",
        dull_yellow("maybeHeight"),
        "or",
        dull_yellow("result"),
        "would work!"
    ]
}
