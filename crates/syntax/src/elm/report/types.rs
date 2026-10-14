//! The reports on types: `toTypeReport` and the reports on records and
//! tuples it leads to.

use nomi_base::diagnostic::code::{Bracket, Next};
use nomi_base::diagnostic::doc::{self, dull_yellow, green, reflow, stack, vcat};
use nomi_base::diagnostic::{Doc, Problem};

use super::{
    Reporter, closing_curly_next, closing_paren_note, convention, curly_before_this,
    earlier_trouble_note, reserved_field, try_adding,
};
use crate::elm::error::{Position, TRecord, TTuple, Type};

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
    pub(super) fn tipe(&self, context: &TypeContext<'_>, tipe: &Type, start: Position) -> Problem {
        match tipe {
            Type::TRecord(record, cur) => self.record_type(context, record, *cur),
            Type::TTuple(tuple, cur) => self.tuple_type(context, tuple, *cur),
            Type::TStart(cur) => self.type_start(context, start, *cur),
            Type::TSpace(space, cur) => self.space(*space, *cur),
            Type::TIndentStart(cur) => self.stuck(
                format!("UNFINISHED {}", context.thing().to_uppercase()),
                start,
                *cur,
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
        }
    }

    /// `TStart`, by what is there.
    fn type_start(&self, context: &TypeContext<'_>, start: Position, cur: Position) -> Problem {
        if let Some(keyword) = self.next_keyword(cur) {
            return self.reserved(
                start,
                cur,
                keyword,
                reflow(
                    "I was expecting to see a type next, but I got stuck on this reserved word:",
                ),
                reflow(&format!(
                    "It looks like you are trying to use `{keyword}` as a type variable, but it is \
                     a reserved word. Try using a different name!"
                )),
            );
        }
        let something = match context {
            TypeContext::Annotation(name) => format!("the `{name}` type annotation"),
            TypeContext::CustomType => "a custom type".to_owned(),
            TypeContext::TypeAlias => "a type alias".to_owned(),
            TypeContext::Port => "a port".to_owned(),
        };
        self.stuck(
            format!("PROBLEM IN {}", context.thing().to_uppercase()),
            start,
            cur,
            reflow(&format!(
                "I was partway through parsing {something}, but I got stuck here:"
            )),
            try_int(),
        )
    }

    /// `toTRecordReport`.
    #[expect(
        clippy::too_many_lines,
        reason = "upstream's `toTRecordReport`, one message per error"
    )]
    fn record_type(&self, context: &TypeContext<'_>, record: &TRecord, start: Position) -> Problem {
        let partway =
            || reflow("I am partway through parsing a record type, but I got stuck here:");
        let unfinished = "UNFINISHED RECORD TYPE";
        match record {
            TRecord::TRecordOpen(cur) => match self.next_keyword(*cur) {
                Some(keyword) => self.reserved(
                    start,
                    *cur,
                    keyword,
                    reflow(
                        "I just started parsing a record type, but I got stuck on this field name:",
                    ),
                    reserved_field(keyword),
                ),
                None => self.stuck(
                    unfinished,
                    start,
                    *cur,
                    reflow("I just started parsing a record type, but I got stuck here:"),
                    fill![
                        "Record types look like",
                        dull_yellow("{ name : String, age : Int },"),
                        "so I was expecting to see a field name next."
                    ],
                ),
            },
            TRecord::TRecordEnd(cur) => {
                self.stuck(unfinished, start, *cur, partway(), curly_before_this())
            }
            TRecord::TRecordField(cur) => self.record_type_field(start, *cur),
            TRecord::TRecordColon(cur) => self.stuck(
                unfinished,
                start,
                *cur,
                partway(),
                stack([
                    fill![
                        "I just saw a field name, so I was expecting to see a colon next. So try \
                         putting an",
                        green(":"),
                        "sign here?"
                    ],
                    note_for_record_type_error(),
                ]),
            ),
            TRecord::TRecordType(tipe, cur) => self.tipe(context, tipe, *cur),
            TRecord::TRecordSpace(space, cur) => self.space(*space, *cur),
            TRecord::TRecordIndentOpen(cur) => self.stuck(
                unfinished,
                start,
                *cur,
                reflow(
                    "I just saw the opening curly brace of a record type, but then I got stuck \
                     here:",
                ),
                stack([
                    fill![
                        "I am expecting a record like",
                        dull_yellow("{ name : String, age : Int }"),
                        "here. Try defining some fields of your own?"
                    ],
                    note_for_record_type_indent_error(),
                ]),
            ),
            TRecord::TRecordIndentEnd(cur) => {
                let partway = "I was partway through parsing a record type, but I got stuck here:";
                self.curly_needs_indent(start, *cur, partway, note_for_record_type_error)
                    .unwrap_or_else(|| {
                        self.stuck(
                            unfinished,
                            start,
                            *cur,
                            reflow(partway),
                            stack([closing_curly_next(), note_for_record_type_indent_error()]),
                        )
                    })
            }
            TRecord::TRecordIndentField(cur) => self.stuck(
                unfinished,
                start,
                *cur,
                reflow(
                    "I am partway through parsing a record type, but I got stuck after that last \
                     comma:",
                ),
                stack([
                    reflow(
                        "Trailing commas are not allowed in record types, so the fix may be to \
                         delete that last comma? Or maybe you were in the middle of defining an \
                         additional field?",
                    ),
                    note_for_record_type_indent_error(),
                ]),
            ),
            TRecord::TRecordIndentColon(cur) => self.stuck(
                unfinished,
                start,
                *cur,
                reflow(
                    "I am partway through parsing a record type. I just saw a record field, so I \
                     was expecting to see a colon next:",
                ),
                stack([
                    fill!["Try putting an", green(":"), "followed by a type?"],
                    note_for_record_type_indent_error(),
                ]),
            ),
            TRecord::TRecordIndentType(cur) => self.stuck(
                unfinished,
                start,
                *cur,
                reflow(
                    "I am partway through parsing a record type, and I was expecting to run into \
                     a type next:",
                ),
                stack([
                    fill![
                        "Try putting something like",
                        dull_yellow("Int"),
                        "or",
                        dull_yellow("String"),
                        "for now?"
                    ],
                    note_for_record_type_indent_error(),
                ]),
            ),
        }
    }

    /// `TRecordField`, by what is there.
    fn record_type_field(&self, start: Position, cur: Position) -> Problem {
        let partway =
            || reflow("I am partway through parsing a record type, but I got stuck here:");
        match self.next(cur) {
            Next::Keyword(keyword) => self.reserved(
                start,
                cur,
                keyword,
                reflow(
                    "I am partway through parsing a record type, but I got stuck on this field \
                     name:",
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
                    note_for_record_type_error(),
                ]),
            ),
            Next::Close(Bracket::Curly) => self.stuck(
                "EXTRA COMMA",
                start,
                cur,
                partway(),
                stack([
                    reflow(
                        "Trailing commas are not allowed in record types. Try deleting the comma \
                         that appears before this closing curly brace.",
                    ),
                    note_for_record_type_error(),
                ]),
            ),
            Next::Operator(_)
            | Next::Close(_)
            | Next::Upper(..)
            | Next::Lower(..)
            | Next::Other(_) => self.stuck(
                "PROBLEM IN RECORD TYPE",
                start,
                cur,
                partway(),
                stack([
                    fill![
                        "I was expecting to see another record field defined next, so I am \
                         looking for a name like",
                        dull_yellow("userName"),
                        "or",
                        dull_yellow("plantHeight").append(Doc::from("."))
                    ],
                    note_for_record_type_error(),
                ]),
            ),
        }
    }

    /// `toTTupleReport`.
    fn tuple_type(&self, context: &TypeContext<'_>, tuple: &TTuple, start: Position) -> Problem {
        let like_maybe_int = || {
            fill![
                "Something like",
                dull_yellow("(Maybe Int)"),
                "or",
                dull_yellow("(List Person)").append(Doc::from(".")),
                "Anything where you are putting parentheses around normal types."
            ]
        };
        let open =
            || reflow("I just saw an open parenthesis, so I was expecting to see a type next.");
        let unfinished = "UNFINISHED PARENTHESES";
        match tuple {
            TTuple::TTupleOpen(cur) => match self.next_keyword(*cur) {
                Some(keyword) => self.reserved(
                    start,
                    *cur,
                    keyword,
                    reflow("I ran into a reserved word unexpectedly:"),
                    reflow(&format!(
                        "It looks like you are trying to use `{keyword}` as a variable name, but \
                         it is a reserved word. Try using a different name!"
                    )),
                ),
                None => self.stuck(unfinished, start, *cur, open(), like_maybe_int()),
            },
            TTuple::TTupleEnd(cur) => self.stuck(
                unfinished,
                start,
                *cur,
                reflow("I was expecting to see a closing parenthesis next, but I got stuck here:"),
                stack([try_adding(Bracket::Paren, '?'), earlier_trouble_note()]),
            ),
            TTuple::TTupleType(tipe, cur) => self.tipe(context, tipe, *cur),
            TTuple::TTupleSpace(space, cur) => self.space(*space, *cur),
            TTuple::TTupleIndentType1(cur) => self.stuck(
                unfinished,
                start,
                *cur,
                open(),
                stack([
                    like_maybe_int(),
                    doc::simple_note(
                        "I can get confused by indentation in cases like this, so maybe you have \
                         a type but it is not indented enough?",
                    ),
                ]),
            ),
            TTuple::TTupleIndentTypeN(cur) => self.stuck(
                "UNFINISHED TUPLE TYPE",
                start,
                *cur,
                reflow(
                    "I think I am in the middle of parsing a tuple type. I just saw a comma, so I \
                     was expecting to see a type next.",
                ),
                stack([
                    fill![
                        "A tuple type looks like",
                        dull_yellow("(Float,Float)"),
                        "or",
                        dull_yellow("(String,Int)").append(Doc::from(",")),
                        "so I think there is a type missing here?"
                    ],
                    doc::simple_note(
                        "I can get confused by indentation in cases like this, so maybe you have \
                         an expression but it is not indented enough?",
                    ),
                ]),
            ),
            TTuple::TTupleIndentEnd(cur) => self.stuck(
                unfinished,
                start,
                *cur,
                reflow("I was expecting to see a closing parenthesis next:"),
                stack([try_adding(Bracket::Paren, '!'), closing_paren_note()]),
            ),
        }
    }
}

fn try_int() -> Doc {
    fill![
        "I was expecting to see a type next. Try putting",
        dull_yellow("Int"),
        "or",
        dull_yellow("String"),
        "for now?"
    ]
}

/// `noteForRecordTypeError` and `noteForRecordTypeIndentError`, by
/// their first paragraph.
fn record_type_note(intro: &str) -> Doc {
    stack([
        doc::simple_note(intro),
        vcat(["{ name : String", ", age : Int", ", height : Float", "}"].map(Doc::from)).indent(4),
        convention('.'),
    ])
}

/// `noteForRecordTypeError`.
fn note_for_record_type_error() -> Doc {
    record_type_note(
        "If you are trying to define a record type across multiple lines, I recommend using \
         this format:",
    )
}

/// `noteForRecordTypeIndentError`.
fn note_for_record_type_indent_error() -> Doc {
    record_type_note(
        "I may be confused by indentation. For example, if you are trying to define a record \
         type across multiple lines, I recommend using this format:",
    )
}
