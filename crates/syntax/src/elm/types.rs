//! `Parse/Type.hs`.

use nomi_base::grow_large;

use super::engine::{
    P, boxed, check_indent, chomp, chomp_and_check_indent, comma_until, fold_more, in_context,
    one_of, one_of_with_fallback, specialize, word1, word2,
};
use super::error::{CustomType, Position, TRecord, TTuple, Type};
use super::literal::{foreign_upper, lower, upper};

/// `term`.
pub(super) fn term(p: &mut P<'_>) -> Result<(), Type> {
    grow_large(|| {
        one_of(
            p,
            Type::TStart,
            &mut [
                &mut |p| foreign_upper(p, Type::TStart),
                &mut |p| lower(p, Type::TStart).map(drop),
                &mut tuple,
                &mut record,
            ],
        )
    })
}

fn tuple(p: &mut P<'_>) -> Result<(), Type> {
    in_context(
        p,
        boxed(Type::TTuple),
        |p| word1(p, b'(', Type::TStart),
        |p| {
            one_of(
                p,
                TTuple::TTupleOpen,
                &mut [&mut |p| word1(p, b')', TTuple::TTupleOpen), &mut |p| {
                    chomp_and_check_indent(p, TTuple::TTupleSpace, TTuple::TTupleIndentType1)?;
                    let end = specialize(p, TTuple::TTupleType, expression)?;
                    check_indent(p, end, TTuple::TTupleIndentEnd)?;
                    comma_until(p, b')', TTuple::TTupleEnd, |p| {
                        chomp_and_check_indent(p, TTuple::TTupleSpace, TTuple::TTupleIndentTypeN)?;
                        let end = specialize(p, TTuple::TTupleType, expression)?;
                        check_indent(p, end, TTuple::TTupleIndentEnd)
                    })
                }],
            )
        },
    )
}

fn record(p: &mut P<'_>) -> Result<(), Type> {
    in_context(
        p,
        boxed(Type::TRecord),
        |p| word1(p, b'{', Type::TStart),
        |p| {
            chomp_and_check_indent(p, TRecord::TRecordSpace, TRecord::TRecordIndentOpen)?;
            one_of(
                p,
                TRecord::TRecordOpen,
                &mut [&mut |p| word1(p, b'}', TRecord::TRecordEnd), &mut |p| {
                    lower(p, TRecord::TRecordField)?;
                    chomp_and_check_indent(p, TRecord::TRecordSpace, TRecord::TRecordIndentColon)?;
                    one_of(
                        p,
                        TRecord::TRecordColon,
                        &mut [
                            &mut |p| {
                                word1(p, b'|', TRecord::TRecordColon)?;
                                chomp_and_check_indent(
                                    p,
                                    TRecord::TRecordSpace,
                                    TRecord::TRecordIndentField,
                                )?;
                                field(p)?;
                                record_end(p)
                            },
                            &mut |p| {
                                word1(p, b':', TRecord::TRecordColon)?;
                                chomp_and_check_indent(
                                    p,
                                    TRecord::TRecordSpace,
                                    TRecord::TRecordIndentType,
                                )?;
                                let end = specialize(p, TRecord::TRecordType, expression)?;
                                check_indent(p, end, TRecord::TRecordIndentEnd)?;
                                record_end(p)
                            },
                        ],
                    )
                }],
            )
        },
    )
}

/// `chompRecordEnd`.
fn record_end(p: &mut P<'_>) -> Result<(), TRecord> {
    comma_until(p, b'}', TRecord::TRecordEnd, |p| {
        chomp_and_check_indent(p, TRecord::TRecordSpace, TRecord::TRecordIndentField)?;
        field(p)
    })
}

/// `chompField`.
fn field(p: &mut P<'_>) -> Result<(), TRecord> {
    lower(p, TRecord::TRecordField)?;
    chomp_and_check_indent(p, TRecord::TRecordSpace, TRecord::TRecordIndentColon)?;
    word1(p, b':', TRecord::TRecordColon)?;
    chomp_and_check_indent(p, TRecord::TRecordSpace, TRecord::TRecordIndentType)?;
    let end = specialize(p, TRecord::TRecordType, expression)?;
    check_indent(p, end, TRecord::TRecordIndentEnd)
}

/// `expression`: a type, and where it ends before the spaces after it.
pub(super) fn expression(p: &mut P<'_>) -> Result<Position, Type> {
    grow_large(|| {
        let end1 = one_of(
            p,
            Type::TStart,
            &mut [&mut app, &mut |p| {
                term(p)?;
                let end = p.cur;
                chomp(p, Type::TSpace)?;
                Ok(end)
            }],
        )?;
        one_of_with_fallback(
            p,
            &mut [&mut |p| {
                check_indent(p, end1, Type::TIndentStart)?;
                word2(p, b'-', b'>', Type::TStart)?;
                chomp_and_check_indent(p, Type::TSpace, Type::TIndentStart)?;
                expression(p)
            }],
            end1,
        )
    })
}

fn app(p: &mut P<'_>) -> Result<Position, Type> {
    foreign_upper(p, Type::TStart)?;
    let upper_end = p.cur;
    chomp(p, Type::TSpace)?;
    chomp_args(p, upper_end)
}

/// `chompArgs`: the arguments of a type, and where the last ends.
fn chomp_args(p: &mut P<'_>, end: Position) -> Result<Position, Type> {
    fold_more(p, end, |p, end| {
        check_indent(p, end, Type::TIndentStart)?;
        term(p)?;
        let new_end = p.cur;
        chomp(p, Type::TSpace)?;
        Ok(new_end)
    })
}

/// `variant`: a constructor of a custom type, and where it ends.
pub(super) fn variant(p: &mut P<'_>) -> Result<Position, CustomType> {
    upper(p, CustomType::CtVariant)?;
    let name_end = p.cur;
    chomp(p, CustomType::CtSpace)?;
    specialize(p, CustomType::CtVariantArg, |p| chomp_args(p, name_end))
}
