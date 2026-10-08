//! `Parse/Pattern.hs`.

use super::engine::{
    P, check_indent, chomp, chomp_and_check_indent, deep, in_context, one_of, one_of_with_fallback,
    specialize, word1, word2,
};
use super::error::{Cursor, PList as L, PRecord as R, PTuple as T, Pattern};
use super::literal::{self, Number, character, foreign_upper, inner_width, keyword, lower, string};

fn tuple(e: T, cur: Cursor) -> Pattern {
    Pattern::PTuple(Box::new(e), cur)
}

fn list(e: L, cur: Cursor) -> Pattern {
    Pattern::PList(Box::new(e), cur)
}

/// `term`.
pub(super) fn term(p: &mut P<'_>) -> Result<(), Pattern> {
    deep(p, |p| {
        one_of(
            p,
            Pattern::PStart,
            &mut [
                &mut record,
                &mut |p| {
                    in_context(
                        p,
                        tuple,
                        |p| word1(p, b'(', Pattern::PStart),
                        |p| {
                            chomp_and_check_indent(p, T::PTupleSpace, T::PTupleIndentExpr1)?;
                            one_of(
                                p,
                                T::PTupleOpen,
                                &mut [
                                    &mut |p| {
                                        let end = specialize(p, T::PTupleExpr, expression)?;
                                        check_indent(p, end, T::PTupleIndentEnd)?;
                                        tuple_end(p)
                                    },
                                    &mut |p| word1(p, b')', T::PTupleEnd),
                                ],
                            )
                        },
                    )
                },
                &mut |p| {
                    in_context(
                        p,
                        list,
                        |p| word1(p, b'[', Pattern::PStart),
                        |p| {
                            chomp_and_check_indent(p, L::PListSpace, L::PListIndentOpen)?;
                            one_of(
                                p,
                                L::PListOpen,
                                &mut [
                                    &mut |p| {
                                        let end = specialize(p, L::PListExpr, expression)?;
                                        check_indent(p, end, L::PListIndentEnd)?;
                                        list_end(p)
                                    },
                                    &mut |p| word1(p, b']', L::PListEnd),
                                ],
                            )
                        },
                    )
                },
                &mut wildcard,
                &mut |p| lower(p, Pattern::PStart).map(drop),
                &mut |p| foreign_upper(p, Pattern::PStart),
                &mut |p| {
                    let start = p.pos;
                    match literal::number(p, Pattern::PStart, Pattern::PNumber)? {
                        Number::Int => Ok(()),
                        Number::Float => {
                            let width = (p.pos - start) as u32;
                            let at = p.cur;
                            let at = Cursor {
                                row: at.row,
                                col: p.word(at.col.wrapping_sub(width)),
                            };
                            p.cerr(Pattern::PFloat(p.word(width), at))
                        }
                    }
                },
                &mut |p| string(p, Pattern::PStart, Pattern::PString),
                &mut |p| character(p, Pattern::PStart, Pattern::PChar),
            ],
        )
    })
}

/// `wildcard`: an underscore, which a name may not go on from.
fn wildcard(p: &mut P<'_>) -> Result<(), Pattern> {
    if p.peek(0) != Some(b'_') {
        return Err(Pattern::PStart(p.cur));
    }
    let (mut pos, mut count) = (p.pos + 1, 0u32);
    loop {
        let width = inner_width(p, pos);
        if width == 0 {
            break;
        }
        pos += width;
        count += 1;
    }
    if count == 0 {
        p.ok_at(p.pos + 1, p.slide(p.cur, 1));
        return Ok(());
    }
    let name = String::from_utf8_lossy(p.bytes(p.pos, pos)).into();
    // 0.19.1 measures from before the underscore, later releases from after.
    let width = if p.v1() { p.word(count + 1) } else { count };
    let at = p.cur;
    p.cerr(Pattern::PWildcardNotVar(name, width, at))
}

fn record(p: &mut P<'_>) -> Result<(), Pattern> {
    in_context(
        p,
        Pattern::PRecord,
        |p| word1(p, b'{', Pattern::PStart),
        |p| {
            chomp_and_check_indent(p, R::PRecordSpace, R::PRecordIndentOpen)?;
            one_of(
                p,
                R::PRecordOpen,
                &mut [
                    &mut |p| {
                        lower(p, R::PRecordField)?;
                        chomp_and_check_indent(p, R::PRecordSpace, R::PRecordIndentEnd)?;
                        record_end(p)
                    },
                    &mut |p| word1(p, b'}', R::PRecordEnd),
                ],
            )
        },
    )
}

fn record_end(p: &mut P<'_>) -> Result<(), R> {
    loop {
        let more = one_of(
            p,
            R::PRecordEnd,
            &mut [
                &mut |p| {
                    word1(p, b',', R::PRecordEnd)?;
                    chomp_and_check_indent(p, R::PRecordSpace, R::PRecordIndentField)?;
                    lower(p, R::PRecordField)?;
                    chomp_and_check_indent(p, R::PRecordSpace, R::PRecordIndentEnd)?;
                    Ok(true)
                },
                &mut |p| word1(p, b'}', R::PRecordEnd).map(|()| false),
            ],
        )?;
        if !more {
            return Ok(());
        }
    }
}

fn tuple_end(p: &mut P<'_>) -> Result<(), T> {
    loop {
        let more = one_of(
            p,
            T::PTupleEnd,
            &mut [
                &mut |p| {
                    word1(p, b',', T::PTupleEnd)?;
                    chomp_and_check_indent(p, T::PTupleSpace, T::PTupleIndentExprN)?;
                    let end = specialize(p, T::PTupleExpr, expression)?;
                    check_indent(p, end, T::PTupleIndentEnd)?;
                    Ok(true)
                },
                &mut |p| word1(p, b')', T::PTupleEnd).map(|()| false),
            ],
        )?;
        if !more {
            return Ok(());
        }
    }
}

fn list_end(p: &mut P<'_>) -> Result<(), L> {
    loop {
        let more = one_of(
            p,
            L::PListEnd,
            &mut [
                &mut |p| {
                    word1(p, b',', L::PListEnd)?;
                    chomp_and_check_indent(p, L::PListSpace, L::PListIndentExpr)?;
                    let end = specialize(p, L::PListExpr, expression)?;
                    check_indent(p, end, L::PListIndentEnd)?;
                    Ok(true)
                },
                &mut |p| word1(p, b']', L::PListEnd).map(|()| false),
            ],
        )?;
        if !more {
            return Ok(());
        }
    }
}

/// `expression`: a pattern, and where it ends before the spaces after it.
pub(super) fn expression(p: &mut P<'_>) -> Result<Cursor, Pattern> {
    deep(p, |p| {
        let mut end = expr_part(p)?;
        loop {
            // `None` to go on after a `::`, the end once there is no more.
            let next = one_of_with_fallback(
                p,
                &mut [
                    &mut |p| {
                        check_indent(p, end, Pattern::PIndentStart)?;
                        word2(p, b':', b':', Pattern::PStart)?;
                        chomp_and_check_indent(p, Pattern::PSpace, Pattern::PIndentStart)?;
                        expr_part(p).map(|end| (false, end))
                    },
                    &mut |p| {
                        check_indent(p, end, Pattern::PIndentStart)?;
                        keyword(p, b"as", Pattern::PStart)?;
                        chomp_and_check_indent(p, Pattern::PSpace, Pattern::PIndentAlias)?;
                        lower(p, Pattern::PAlias)?;
                        let new_end = p.cur;
                        chomp(p, Pattern::PSpace)?;
                        Ok((true, new_end))
                    },
                ],
                (true, end),
            )?;
            end = next.1;
            if next.0 {
                return Ok(end);
            }
        }
    })
}

/// `exprPart`: a term, or a constructor and its arguments.
fn expr_part(p: &mut P<'_>) -> Result<Cursor, Pattern> {
    one_of(
        p,
        Pattern::PStart,
        &mut [
            &mut |p| {
                foreign_upper(p, Pattern::PStart)?;
                let mut end = p.cur;
                chomp(p, Pattern::PSpace)?;
                loop {
                    let more = one_of_with_fallback(
                        p,
                        &mut [&mut |p| {
                            check_indent(p, end, Pattern::PIndentStart)?;
                            term(p).map(|()| true)
                        }],
                        false,
                    )?;
                    if !more {
                        return Ok(end);
                    }
                    end = p.cur;
                    chomp(p, Pattern::PSpace)?;
                }
            },
            &mut |p| {
                term(p)?;
                let end = p.cur;
                chomp(p, Pattern::PSpace)?;
                Ok(end)
            },
        ],
    )
}
