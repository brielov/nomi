//! `Parse/Type.hs`.

use super::engine::{
    P, check_indent, chomp, chomp_and_check_indent, deep, in_context, one_of, one_of_with_fallback,
    specialize, word1, word2,
};
use super::error::{Cursor, CustomType as CT, TRecord as R, TTuple as T, Type};
use super::literal::{foreign_upper, lower, upper};

fn tuple(e: T, cur: Cursor) -> Type {
    Type::TTuple(Box::new(e), cur)
}

fn record(e: R, cur: Cursor) -> Type {
    Type::TRecord(Box::new(e), cur)
}

/// `term`.
pub(super) fn term(p: &mut P<'_>) -> Result<(), Type> {
    deep(p, |p| {
        one_of(
            p,
            Type::TStart,
            &mut [
                &mut |p| foreign_upper(p, Type::TStart),
                &mut |p| lower(p, Type::TStart).map(drop),
                &mut |p| {
                    in_context(
                        p,
                        tuple,
                        |p| word1(p, b'(', Type::TStart),
                        |p| {
                            one_of(
                                p,
                                T::TTupleOpen,
                                &mut [&mut |p| word1(p, b')', T::TTupleOpen), &mut |p| {
                                    chomp_and_check_indent(
                                        p,
                                        T::TTupleSpace,
                                        T::TTupleIndentType1,
                                    )?;
                                    let end = specialize(p, T::TTupleType, expression)?;
                                    check_indent(p, end, T::TTupleIndentEnd)?;
                                    chomp_tuple_end(p)
                                }],
                            )
                        },
                    )
                },
                &mut |p| {
                    in_context(
                        p,
                        record,
                        |p| word1(p, b'{', Type::TStart),
                        |p| {
                            chomp_and_check_indent(p, R::TRecordSpace, R::TRecordIndentOpen)?;
                            one_of(
                                p,
                                R::TRecordOpen,
                                &mut [&mut |p| word1(p, b'}', R::TRecordEnd), &mut |p| {
                                    lower(p, R::TRecordField)?;
                                    chomp_and_check_indent(
                                        p,
                                        R::TRecordSpace,
                                        R::TRecordIndentColon,
                                    )?;
                                    one_of(
                                        p,
                                        R::TRecordColon,
                                        &mut [
                                            &mut |p| {
                                                word1(p, b'|', R::TRecordColon)?;
                                                chomp_and_check_indent(
                                                    p,
                                                    R::TRecordSpace,
                                                    R::TRecordIndentField,
                                                )?;
                                                chomp_field(p)?;
                                                chomp_record_end(p)
                                            },
                                            &mut |p| {
                                                word1(p, b':', R::TRecordColon)?;
                                                chomp_and_check_indent(
                                                    p,
                                                    R::TRecordSpace,
                                                    R::TRecordIndentType,
                                                )?;
                                                let end =
                                                    specialize(p, R::TRecordType, expression)?;
                                                check_indent(p, end, R::TRecordIndentEnd)?;
                                                chomp_record_end(p)
                                            },
                                        ],
                                    )
                                }],
                            )
                        },
                    )
                },
            ],
        )
    })
}

/// `expression`: a type, and where it ends before the spaces after it.
pub(super) fn expression(p: &mut P<'_>) -> Result<Cursor, Type> {
    deep(p, |p| {
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

fn app(p: &mut P<'_>) -> Result<Cursor, Type> {
    foreign_upper(p, Type::TStart)?;
    let upper_end = p.cur;
    chomp(p, Type::TSpace)?;
    chomp_args(p, upper_end)
}

/// `chompArgs`: the arguments of a type, and where the last ends.
fn chomp_args(p: &mut P<'_>, mut end: Cursor) -> Result<Cursor, Type> {
    loop {
        let next = one_of_with_fallback(
            p,
            &mut [&mut |p| {
                check_indent(p, end, Type::TIndentStart)?;
                term(p)?;
                let new_end = p.cur;
                chomp(p, Type::TSpace)?;
                Ok(Some(new_end))
            }],
            None,
        )?;
        match next {
            Some(new_end) => end = new_end,
            None => return Ok(end),
        }
    }
}

fn chomp_tuple_end(p: &mut P<'_>) -> Result<(), T> {
    loop {
        let more = one_of(
            p,
            T::TTupleEnd,
            &mut [
                &mut |p| {
                    word1(p, b',', T::TTupleEnd)?;
                    chomp_and_check_indent(p, T::TTupleSpace, T::TTupleIndentTypeN)?;
                    let end = specialize(p, T::TTupleType, expression)?;
                    check_indent(p, end, T::TTupleIndentEnd)?;
                    Ok(true)
                },
                &mut |p| word1(p, b')', T::TTupleEnd).map(|()| false),
            ],
        )?;
        if !more {
            return Ok(());
        }
    }
}

fn chomp_record_end(p: &mut P<'_>) -> Result<(), R> {
    loop {
        let more = one_of(
            p,
            R::TRecordEnd,
            &mut [
                &mut |p| {
                    word1(p, b',', R::TRecordEnd)?;
                    chomp_and_check_indent(p, R::TRecordSpace, R::TRecordIndentField)?;
                    chomp_field(p)?;
                    Ok(true)
                },
                &mut |p| word1(p, b'}', R::TRecordEnd).map(|()| false),
            ],
        )?;
        if !more {
            return Ok(());
        }
    }
}

fn chomp_field(p: &mut P<'_>) -> Result<(), R> {
    lower(p, R::TRecordField)?;
    chomp_and_check_indent(p, R::TRecordSpace, R::TRecordIndentColon)?;
    word1(p, b':', R::TRecordColon)?;
    chomp_and_check_indent(p, R::TRecordSpace, R::TRecordIndentType)?;
    let end = specialize(p, R::TRecordType, expression)?;
    check_indent(p, end, R::TRecordIndentEnd)
}

/// `variant`: a constructor of a custom type, and where it ends.
pub(super) fn variant(p: &mut P<'_>) -> Result<Cursor, CT> {
    upper(p, CT::CtVariant)?;
    let name_end = p.cur;
    chomp(p, CT::CtSpace)?;
    specialize(p, CT::CtVariantArg, |p| chomp_args(p, name_end))
}
