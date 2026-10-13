//! `Parse/Pattern.hs`.

use std::ops::ControlFlow;

use nomi_base::grow_large;

use super::engine::{
    P, boxed, check_indent, chomp, chomp_and_check_indent, comma_until, fold_more, in_context,
    one_of, one_of_with_fallback, specialize, word1, word2,
};
use super::error::{PList, PRecord, PTuple, Pattern, Position};
use super::literal::{
    self, Number, character, chomp_inners, foreign_upper, keyword, lower, string,
};

/// `term`.
pub(super) fn term(p: &mut P<'_>) -> Result<(), Pattern> {
    grow_large(|| {
        one_of(
            p,
            Pattern::PStart,
            &mut [
                &mut record,
                &mut tuple,
                &mut list,
                &mut wildcard,
                &mut |p| lower(p, Pattern::PStart).map(drop),
                &mut |p| foreign_upper(p, Pattern::PStart),
                &mut |p| {
                    let start = p.pos;
                    match literal::number(p, Pattern::PStart, Pattern::PNumber)? {
                        Number::Int => Ok(()),
                        Number::Float => {
                            let width = (p.pos - start) as u32;
                            let at = Position {
                                row: p.cur.row,
                                col: p.wrap(p.cur.col.wrapping_sub(width)),
                            };
                            p.cerr(Pattern::PFloat(p.wrap(width), at))
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
    let underscore = p.here().step(p, 1, 1);
    let end = chomp_inners(p, underscore);
    if end.pos == underscore.pos {
        p.ok_at(underscore);
        return Ok(());
    }
    let name = literal::name(p, p.pos, end.pos);
    // 0.19.1 measures from before the underscore, later releases from after.
    let from = if p.v1() { p.cur } else { underscore.cur };
    let width = p.wrap(end.cur.col.wrapping_sub(from.col));
    let at = p.cur;
    p.cerr(Pattern::PWildcardNotVar { name, width, at })
}

fn record(p: &mut P<'_>) -> Result<(), Pattern> {
    in_context(
        p,
        Pattern::PRecord,
        |p| word1(p, b'{', Pattern::PStart),
        |p| {
            chomp_and_check_indent(p, PRecord::PRecordSpace, PRecord::PRecordIndentOpen)?;
            one_of(
                p,
                PRecord::PRecordOpen,
                &mut [
                    &mut |p| {
                        lower(p, PRecord::PRecordField)?;
                        chomp_and_check_indent(
                            p,
                            PRecord::PRecordSpace,
                            PRecord::PRecordIndentEnd,
                        )?;
                        comma_until(p, b'}', PRecord::PRecordEnd, |p| {
                            chomp_and_check_indent(
                                p,
                                PRecord::PRecordSpace,
                                PRecord::PRecordIndentField,
                            )?;
                            lower(p, PRecord::PRecordField)?;
                            chomp_and_check_indent(
                                p,
                                PRecord::PRecordSpace,
                                PRecord::PRecordIndentEnd,
                            )
                        })
                    },
                    &mut |p| word1(p, b'}', PRecord::PRecordEnd),
                ],
            )
        },
    )
}

fn tuple(p: &mut P<'_>) -> Result<(), Pattern> {
    in_context(
        p,
        boxed(Pattern::PTuple),
        |p| word1(p, b'(', Pattern::PStart),
        |p| {
            chomp_and_check_indent(p, PTuple::PTupleSpace, PTuple::PTupleIndentExpr1)?;
            one_of(
                p,
                PTuple::PTupleOpen,
                &mut [
                    &mut |p| {
                        let end = specialize(p, PTuple::PTupleExpr, expression)?;
                        check_indent(p, end, PTuple::PTupleIndentEnd)?;
                        comma_until(p, b')', PTuple::PTupleEnd, |p| {
                            chomp_and_check_indent(
                                p,
                                PTuple::PTupleSpace,
                                PTuple::PTupleIndentExprN,
                            )?;
                            let end = specialize(p, PTuple::PTupleExpr, expression)?;
                            check_indent(p, end, PTuple::PTupleIndentEnd)
                        })
                    },
                    &mut |p| word1(p, b')', PTuple::PTupleEnd),
                ],
            )
        },
    )
}

fn list(p: &mut P<'_>) -> Result<(), Pattern> {
    in_context(
        p,
        boxed(Pattern::PList),
        |p| word1(p, b'[', Pattern::PStart),
        |p| {
            chomp_and_check_indent(p, PList::PListSpace, PList::PListIndentOpen)?;
            one_of(
                p,
                PList::PListOpen,
                &mut [
                    &mut |p| {
                        let end = specialize(p, PList::PListExpr, expression)?;
                        check_indent(p, end, PList::PListIndentEnd)?;
                        comma_until(p, b']', PList::PListEnd, |p| {
                            chomp_and_check_indent(p, PList::PListSpace, PList::PListIndentExpr)?;
                            let end = specialize(p, PList::PListExpr, expression)?;
                            check_indent(p, end, PList::PListIndentEnd)
                        })
                    },
                    &mut |p| word1(p, b']', PList::PListEnd),
                ],
            )
        },
    )
}

/// `expression`: a pattern, and where it ends before the spaces after it.
pub(super) fn expression(p: &mut P<'_>) -> Result<Position, Pattern> {
    grow_large(|| {
        let mut end = expr_part(p)?;
        loop {
            let step = one_of_with_fallback(
                p,
                &mut [
                    &mut |p| {
                        check_indent(p, end, Pattern::PIndentStart)?;
                        word2(p, b':', b':', Pattern::PStart)?;
                        chomp_and_check_indent(p, Pattern::PSpace, Pattern::PIndentStart)?;
                        expr_part(p).map(ControlFlow::Continue)
                    },
                    &mut |p| {
                        check_indent(p, end, Pattern::PIndentStart)?;
                        keyword(p, b"as", Pattern::PStart)?;
                        chomp_and_check_indent(p, Pattern::PSpace, Pattern::PIndentAlias)?;
                        lower(p, Pattern::PAlias)?;
                        let alias_end = p.cur;
                        chomp(p, Pattern::PSpace)?;
                        Ok(ControlFlow::Break(alias_end))
                    },
                ],
                ControlFlow::Break(end),
            )?;
            match step {
                ControlFlow::Continue(new_end) => end = new_end,
                ControlFlow::Break(end) => return Ok(end),
            }
        }
    })
}

/// `exprPart`: a term, or a constructor and its arguments.
fn expr_part(p: &mut P<'_>) -> Result<Position, Pattern> {
    one_of(
        p,
        Pattern::PStart,
        &mut [
            &mut |p| {
                foreign_upper(p, Pattern::PStart)?;
                let end = p.cur;
                chomp(p, Pattern::PSpace)?;
                fold_more(p, end, |p, end| {
                    check_indent(p, end, Pattern::PIndentStart)?;
                    term(p)?;
                    let new_end = p.cur;
                    chomp(p, Pattern::PSpace)?;
                    Ok(new_end)
                })
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
