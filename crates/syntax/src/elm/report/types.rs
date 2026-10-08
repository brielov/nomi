//! The reports on types: `toTypeReport` and the reports on records and
//! tuples it leads to.

use nomi_base::diagnostic::code::{Next, Region};
use nomi_base::diagnostic::doc::{self, reflow, stack, vcat};
use nomi_base::diagnostic::{Doc, Problem};

use super::{Reporter, dullyellow, green, surroundings, to_region};
use crate::elm::error::{Cursor, TRecord, TTuple, Type};

/// `TContext`: what the type is part of.
pub(super) enum TypeContext<'a> {
    Annotation(&'a str),
    CustomType,
    TypeAlias,
    Port,
}

impl TypeContext<'_> {
    fn thing(&self) -> &'static str {
        match self {
            TypeContext::Annotation(_) => "type annotation",
            TypeContext::CustomType => "custom type",
            TypeContext::TypeAlias => "type alias",
            TypeContext::Port => "port",
        }
    }
}

impl Reporter<'_, '_> {
    /// `toTypeReport`.
    pub(super) fn tipe(
        &self,
        context: &TypeContext<'_>,
        tipe: &Type,
        start: Cursor,
    ) -> Option<Problem> {
        let try_int = || {
            fill![
                "I",
                "was",
                "expecting",
                "to",
                "see",
                "a",
                "type",
                "next.",
                "Try",
                "putting",
                dullyellow("Int"),
                "or",
                dullyellow("String"),
                "for",
                "now?"
            ]
        };
        Some(match tipe {
            Type::TRecord(record, cur) => self.record_type(context, record, *cur)?,
            Type::TTuple(tuple, cur) => self.tuple_type(context, tuple, *cur)?,
            Type::TStart(cur) => match self.next(*cur) {
                Next::Keyword(keyword) => self.within(
                    "RESERVED WORD",
                    surroundings(start, *cur),
                    self.keyword_region(*cur, keyword),
                    reflow(
                        "I was expecting to see a type next, but I got stuck on this reserved \
                         word:",
                    ),
                    reflow(&format!(
                        "It looks like you are trying to use `{keyword}` as a type variable, \
                         but it is a reserved word. Try using a different name!"
                    )),
                ),
                _ => {
                    let something = match context {
                        TypeContext::Annotation(name) => format!("the `{name}` type annotation"),
                        TypeContext::CustomType => "a custom type".to_owned(),
                        TypeContext::TypeAlias => "a type alias".to_owned(),
                        TypeContext::Port => "a port".to_owned(),
                    };
                    self.within(
                        format!("PROBLEM IN {}", context.thing().to_uppercase()),
                        surroundings(start, *cur),
                        to_region(*cur),
                        reflow(&format!(
                            "I was partway through parsing {something}, but I got stuck here:"
                        )),
                        try_int(),
                    )
                }
            },
            Type::TSpace(space, cur) => self.space(*space, *cur),
            Type::TIndentStart(cur) => self.within(
                format!("UNFINISHED {}", context.thing().to_uppercase()),
                surroundings(start, *cur),
                to_region(*cur),
                reflow(&format!(
                    "I was partway through parsing a {}, but I got stuck here:",
                    context.thing()
                )),
                stack([
                    try_int(),
                    doc::simple_note(
                        "I can get confused by indentation. If you think there is already a \
                         type next, maybe it is not indented enough?",
                    ),
                ]),
            ),
        })
    }

    /// `toTRecordReport`.
    fn record_type(
        &self,
        context: &TypeContext<'_>,
        record: &TRecord,
        start: Cursor,
    ) -> Option<Problem> {
        let report = |title: &'static str, cur: Cursor, before: &str, after: Doc| {
            self.within(
                title,
                surroundings(start, cur),
                to_region(cur),
                reflow(before),
                after,
            )
        };
        let partway = "I am partway through parsing a record type, but I got stuck here:";
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
        Some(match record {
            TRecord::TRecordOpen(cur) => match self.next(*cur) {
                Next::Keyword(keyword) => reserved(
                    *cur,
                    keyword,
                    "I just started parsing a record type, but I got stuck on this field name:",
                ),
                _ => report(
                    "UNFINISHED RECORD TYPE",
                    *cur,
                    "I just started parsing a record type, but I got stuck here:",
                    fill![
                        "Record",
                        "types",
                        "look",
                        "like",
                        dullyellow("{ name : String, age : Int },"),
                        "so",
                        "I",
                        "was",
                        "expecting",
                        "to",
                        "see",
                        "a",
                        "field",
                        "name",
                        "next."
                    ],
                ),
            },
            TRecord::TRecordEnd(cur) => report(
                "UNFINISHED RECORD TYPE",
                *cur,
                partway,
                stack([
                    fill![
                        "I",
                        "was",
                        "expecting",
                        "to",
                        "see",
                        "a",
                        "closing",
                        "curly",
                        "brace",
                        "before",
                        "this,",
                        "so",
                        "try",
                        "adding",
                        "a",
                        dullyellow("}"),
                        "and",
                        "see",
                        "if",
                        "that",
                        "helps?"
                    ],
                    doc::simple_note(
                        "When I get stuck like this, it usually means that there is a missing \
                         parenthesis or bracket somewhere earlier. It could also be a stray \
                         keyword or operator.",
                    ),
                ]),
            ),
            TRecord::TRecordField(cur) => match self.next(*cur) {
                Next::Keyword(keyword) => reserved(
                    *cur,
                    keyword,
                    "I am partway through parsing a record type, but I got stuck on this field \
                     name:",
                ),
                Next::Other(Some(',')) => report(
                    "EXTRA COMMA",
                    *cur,
                    partway,
                    stack([
                        reflow("I am seeing two commas in a row. This is the second one!"),
                        reflow("Just delete one of the commas and you should be all set!"),
                        note_for_record_type_error(),
                    ]),
                ),
                Next::Close(_, '}') => report(
                    "EXTRA COMMA",
                    *cur,
                    partway,
                    stack([
                        reflow(
                            "Trailing commas are not allowed in record types. Try deleting the \
                             comma that appears before this closing curly brace.",
                        ),
                        note_for_record_type_error(),
                    ]),
                ),
                _ => report(
                    "PROBLEM IN RECORD TYPE",
                    *cur,
                    partway,
                    stack([
                        fill![
                            "I",
                            "was",
                            "expecting",
                            "to",
                            "see",
                            "another",
                            "record",
                            "field",
                            "defined",
                            "next,",
                            "so",
                            "I",
                            "am",
                            "looking",
                            "for",
                            "a",
                            "name",
                            "like",
                            dullyellow("userName"),
                            "or",
                            dullyellow("plantHeight").append(Doc::from("."))
                        ],
                        note_for_record_type_error(),
                    ]),
                ),
            },
            TRecord::TRecordColon(cur) => report(
                "UNFINISHED RECORD TYPE",
                *cur,
                partway,
                stack([
                    fill![
                        "I",
                        "just",
                        "saw",
                        "a",
                        "field",
                        "name,",
                        "so",
                        "I",
                        "was",
                        "expecting",
                        "to",
                        "see",
                        "a",
                        "colon",
                        "next.",
                        "So",
                        "try",
                        "putting",
                        "an",
                        green(":"),
                        "sign",
                        "here?"
                    ],
                    note_for_record_type_error(),
                ]),
            ),
            TRecord::TRecordType(tipe, cur) => return self.tipe(context, tipe, *cur),
            TRecord::TRecordSpace(space, cur) => self.space(*space, *cur),
            TRecord::TRecordIndentOpen(cur) => report(
                "UNFINISHED RECORD TYPE",
                *cur,
                "I just saw the opening curly brace of a record type, but then I got stuck here:",
                stack([
                    fill![
                        "I",
                        "am",
                        "expecting",
                        "a",
                        "record",
                        "like",
                        dullyellow("{ name : String, age : Int }"),
                        "here.",
                        "Try",
                        "defining",
                        "some",
                        "fields",
                        "of",
                        "your",
                        "own?"
                    ],
                    note_for_record_type_indent_error(),
                ]),
            ),
            TRecord::TRecordIndentEnd(cur) => {
                let partway = "I was partway through parsing a record type, but I got stuck here:";
                match (self.source)
                    .next_line_starts_with_close_curly(super::position(*cur), self.compat)
                {
                    Some(curly) => self.within(
                        "NEED MORE INDENTATION",
                        Region {
                            start: super::position(start),
                            end: curly,
                        },
                        Region {
                            start: curly,
                            end: curly,
                        },
                        reflow(partway),
                        stack([
                            reflow(
                                "I need this curly brace to be indented more. Try adding \
                                     some spaces before it!",
                            ),
                            note_for_record_type_error(),
                        ]),
                    ),
                    None => report(
                        "UNFINISHED RECORD TYPE",
                        *cur,
                        partway,
                        stack([
                            fill![
                                "I",
                                "was",
                                "expecting",
                                "to",
                                "see",
                                "a",
                                "closing",
                                "curly",
                                "brace",
                                "next.",
                                "Try",
                                "putting",
                                "a",
                                green("}"),
                                "next",
                                "and",
                                "see",
                                "if",
                                "that",
                                "helps?"
                            ],
                            note_for_record_type_indent_error(),
                        ]),
                    ),
                }
            }
            TRecord::TRecordIndentField(cur) => report(
                "UNFINISHED RECORD TYPE",
                *cur,
                "I am partway through parsing a record type, but I got stuck after that last \
                 comma:",
                stack([
                    reflow(
                        "Trailing commas are not allowed in record types, so the fix may be to \
                         delete that last comma? Or maybe you were in the middle of defining an \
                         additional field?",
                    ),
                    note_for_record_type_indent_error(),
                ]),
            ),
            TRecord::TRecordIndentColon(cur) => report(
                "UNFINISHED RECORD TYPE",
                *cur,
                "I am partway through parsing a record type. I just saw a record field, so I was \
                 expecting to see a colon next:",
                stack([
                    fill![
                        "Try",
                        "putting",
                        "an",
                        green(":"),
                        "followed",
                        "by",
                        "a",
                        "type?"
                    ],
                    note_for_record_type_indent_error(),
                ]),
            ),
            TRecord::TRecordIndentType(cur) => report(
                "UNFINISHED RECORD TYPE",
                *cur,
                "I am partway through parsing a record type, and I was expecting to run into a \
                 type next:",
                stack([
                    fill![
                        "Try",
                        "putting",
                        "something",
                        "like",
                        dullyellow("Int"),
                        "or",
                        dullyellow("String"),
                        "for",
                        "now?"
                    ],
                    note_for_record_type_indent_error(),
                ]),
            ),
        })
    }

    /// `toTTupleReport`.
    fn tuple_type(
        &self,
        context: &TypeContext<'_>,
        tuple: &TTuple,
        start: Cursor,
    ) -> Option<Problem> {
        let report = |title: &'static str, cur: Cursor, before: &str, after: Doc| {
            self.within(
                title,
                surroundings(start, cur),
                to_region(cur),
                reflow(before),
                after,
            )
        };
        let like_maybe_int = || {
            fill![
                "Something",
                "like",
                dullyellow("(Maybe Int)"),
                "or",
                dullyellow("(List Person)").append(Doc::from(".")),
                "Anything",
                "where",
                "you",
                "are",
                "putting",
                "parentheses",
                "around",
                "normal",
                "types."
            ]
        };
        let open = "I just saw an open parenthesis, so I was expecting to see a type next.";
        Some(match tuple {
            TTuple::TTupleOpen(cur) => match self.next(*cur) {
                Next::Keyword(keyword) => self.within(
                    "RESERVED WORD",
                    surroundings(start, *cur),
                    self.keyword_region(*cur, keyword),
                    reflow("I ran into a reserved word unexpectedly:"),
                    reflow(&format!(
                        "It looks like you are trying to use `{keyword}` as a variable name, but \
                         it is a reserved word. Try using a different name!"
                    )),
                ),
                _ => report("UNFINISHED PARENTHESES", *cur, open, like_maybe_int()),
            },
            TTuple::TTupleEnd(cur) => report(
                "UNFINISHED PARENTHESES",
                *cur,
                "I was expecting to see a closing parenthesis next, but I got stuck here:",
                stack([
                    fill![
                        "Try",
                        "adding",
                        "a",
                        dullyellow(")"),
                        "to",
                        "see",
                        "if",
                        "that",
                        "helps?"
                    ],
                    doc::simple_note(
                        "I can get stuck when I run into keywords, operators, parentheses, or \
                         brackets unexpectedly. So there may be some earlier syntax trouble \
                         (like extra parenthesis or missing brackets) that is confusing me.",
                    ),
                ]),
            ),
            TTuple::TTupleType(tipe, cur) => return self.tipe(context, tipe, *cur),
            TTuple::TTupleSpace(space, cur) => self.space(*space, *cur),
            TTuple::TTupleIndentType1(cur) => report(
                "UNFINISHED PARENTHESES",
                *cur,
                open,
                stack([
                    like_maybe_int(),
                    doc::simple_note(
                        "I can get confused by indentation in cases like this, so maybe you \
                         have a type but it is not indented enough?",
                    ),
                ]),
            ),
            TTuple::TTupleIndentTypeN(cur) => report(
                "UNFINISHED TUPLE TYPE",
                *cur,
                "I think I am in the middle of parsing a tuple type. I just saw a comma, so I \
                 was expecting to see a type next.",
                stack([
                    fill![
                        "A",
                        "tuple",
                        "type",
                        "looks",
                        "like",
                        dullyellow("(Float,Float)"),
                        "or",
                        dullyellow("(String,Int)").append(Doc::from(",")),
                        "so",
                        "I",
                        "think",
                        "there",
                        "is",
                        "a",
                        "type",
                        "missing",
                        "here?"
                    ],
                    doc::simple_note(
                        "I can get confused by indentation in cases like this, so maybe you \
                         have an expression but it is not indented enough?",
                    ),
                ]),
            ),
            TTuple::TTupleIndentEnd(cur) => report(
                "UNFINISHED PARENTHESES",
                *cur,
                "I was expecting to see a closing parenthesis next:",
                stack([
                    fill![
                        "Try",
                        "adding",
                        "a",
                        dullyellow(")"),
                        "to",
                        "see",
                        "if",
                        "that",
                        "helps!"
                    ],
                    doc::simple_note(
                        "I can get confused by indentation in cases like this, so maybe you \
                         have a closing parenthesis but it is not indented enough?",
                    ),
                ]),
            ),
        })
    }
}

/// `noteForRecordTypeError` and `noteForRecordTypeIndentError`, by
/// their first paragraph.
fn record_type_note(intro: &str) -> Doc {
    stack([
        doc::simple_note(intro),
        vcat(["{ name : String", ", age : Int", ", height : Float", "}"].map(Doc::from)).indent(4),
        reflow(
            "Notice that each line starts with some indentation. Usually two or four spaces. \
             This is the stylistic convention in the Elm ecosystem.",
        ),
    ])
}

fn note_for_record_type_error() -> Doc {
    record_type_note(
        "If you are trying to define a record type across multiple lines, I recommend using \
         this format:",
    )
}

fn note_for_record_type_indent_error() -> Doc {
    record_type_note(
        "I may be confused by indentation. For example, if you are trying to define a record \
         type across multiple lines, I recommend using this format:",
    )
}
