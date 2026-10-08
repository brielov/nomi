//! `Parse/Expression.hs`.

use super::engine::{
    P, check_aligned, check_indent, chomp, chomp_and_check_indent, deep, in_context, one_of,
    one_of_with_fallback, specialize, with_backset_indent, with_indent, word1, word2,
};
use super::error::{Case, Cursor, Def, Destruct, Expr, Func, If, Let, List, Name, Record, Tuple};
use super::literal::{self, character, foreign_alpha, keyword, lower, operator, string};
use super::{pattern, types};

fn boxed<X, Y>(wrap: fn(Box<X>, Cursor) -> Y) -> impl FnOnce(X, Cursor) -> Y {
    move |x, cur| wrap(Box::new(x), cur)
}

// TERMS

/// `term`.
pub(super) fn term(p: &mut P<'_>) -> Result<(), Expr> {
    deep(p, |p| {
        one_of(
            p,
            Expr::Start,
            &mut [
                &mut |p| {
                    foreign_alpha(p, Expr::Start)?;
                    accessible(p)
                },
                &mut |p| string(p, Expr::Start, Expr::String),
                &mut |p| literal::number(p, Expr::Start, Expr::Number).map(drop),
                &mut literal::shader,
                &mut list,
                &mut |p| {
                    record(p)?;
                    accessible(p)
                },
                &mut |p| {
                    tuple(p)?;
                    accessible(p)
                },
                &mut |p| {
                    word1(p, b'.', Expr::Dot)?;
                    lower(p, Expr::Access).map(drop)
                },
                &mut |p| character(p, Expr::Start, Expr::Char),
            ],
        )
    })
}

/// `accessible`: the fields taken of what was just read.
fn accessible(p: &mut P<'_>) -> Result<(), Expr> {
    loop {
        let more = one_of_with_fallback(
            p,
            &mut [&mut |p| {
                word1(p, b'.', Expr::Dot)?;
                lower(p, Expr::Access).map(|_| true)
            }],
            false,
        )?;
        if !more {
            return Ok(());
        }
    }
}

// LISTS

fn list(p: &mut P<'_>) -> Result<(), Expr> {
    in_context(
        p,
        boxed(Expr::List),
        |p| word1(p, b'[', Expr::Start),
        |p| {
            chomp_and_check_indent(p, List::ListSpace, List::ListIndentOpen)?;
            one_of(
                p,
                List::ListOpen,
                &mut [
                    &mut |p| {
                        let end = specialize(p, List::ListExpr, expression)?;
                        check_indent(p, end, List::ListIndentEnd)?;
                        list_end(p)
                    },
                    &mut |p| word1(p, b']', List::ListOpen),
                ],
            )
        },
    )
}

fn list_end(p: &mut P<'_>) -> Result<(), List> {
    loop {
        let more = one_of(
            p,
            List::ListEnd,
            &mut [
                &mut |p| {
                    word1(p, b',', List::ListEnd)?;
                    chomp_and_check_indent(p, List::ListSpace, List::ListIndentExpr)?;
                    let end = specialize(p, List::ListExpr, expression)?;
                    check_indent(p, end, List::ListIndentEnd)?;
                    Ok(true)
                },
                &mut |p| word1(p, b']', List::ListEnd).map(|()| false),
            ],
        )?;
        if !more {
            return Ok(());
        }
    }
}

// TUPLES

fn tuple(p: &mut P<'_>) -> Result<(), Expr> {
    in_context(
        p,
        boxed(Expr::Tuple),
        |p| word1(p, b'(', Expr::Start),
        |p| {
            let before = p.cur;
            chomp_and_check_indent(p, Tuple::TupleSpace, Tuple::TupleIndentExpr1)?;
            if p.cur != before {
                return first_entry(p);
            }
            one_of(
                p,
                Tuple::TupleIndentExpr1,
                &mut [
                    &mut |p| {
                        let op =
                            operator(p, Tuple::TupleIndentExpr1, Tuple::TupleOperatorReserved)?;
                        if &*op != "-" {
                            return word1(p, b')', Tuple::TupleOperatorClose);
                        }
                        one_of(
                            p,
                            Tuple::TupleOperatorClose,
                            &mut [
                                &mut |p| word1(p, b')', Tuple::TupleOperatorClose),
                                &mut |p| {
                                    let end = specialize(p, Tuple::TupleExpr, |p| {
                                        term(p)?;
                                        let end = p.cur;
                                        chomp(p, Expr::Space)?;
                                        expr_end(p, end)
                                    })?;
                                    check_indent(p, end, Tuple::TupleIndentEnd)?;
                                    tuple_end(p)
                                },
                            ],
                        )
                    },
                    &mut |p| word1(p, b')', Tuple::TupleIndentExpr1),
                    &mut first_entry,
                ],
            )
        },
    )
}

fn first_entry(p: &mut P<'_>) -> Result<(), Tuple> {
    let end = specialize(p, Tuple::TupleExpr, expression)?;
    check_indent(p, end, Tuple::TupleIndentEnd)?;
    tuple_end(p)
}

fn tuple_end(p: &mut P<'_>) -> Result<(), Tuple> {
    loop {
        let more = one_of(
            p,
            Tuple::TupleEnd,
            &mut [
                &mut |p| {
                    word1(p, b',', Tuple::TupleEnd)?;
                    chomp_and_check_indent(p, Tuple::TupleSpace, Tuple::TupleIndentExprN)?;
                    let end = specialize(p, Tuple::TupleExpr, expression)?;
                    check_indent(p, end, Tuple::TupleIndentEnd)?;
                    Ok(true)
                },
                &mut |p| word1(p, b')', Tuple::TupleEnd).map(|()| false),
            ],
        )?;
        if !more {
            return Ok(());
        }
    }
}

// RECORDS

fn record(p: &mut P<'_>) -> Result<(), Expr> {
    in_context(
        p,
        boxed(Expr::Record),
        |p| word1(p, b'{', Expr::Start),
        |p| {
            chomp_and_check_indent(p, Record::RecordSpace, Record::RecordIndentOpen)?;
            one_of(
                p,
                Record::RecordOpen,
                &mut [&mut |p| word1(p, b'}', Record::RecordOpen), &mut |p| {
                    lower(p, Record::RecordField)?;
                    chomp_and_check_indent(p, Record::RecordSpace, Record::RecordIndentEquals)?;
                    one_of(
                        p,
                        Record::RecordEquals,
                        &mut [
                            &mut |p| {
                                word1(p, b'|', Record::RecordEquals)?;
                                chomp_and_check_indent(
                                    p,
                                    Record::RecordSpace,
                                    Record::RecordIndentField,
                                )?;
                                field(p)?;
                                fields(p)
                            },
                            &mut |p| {
                                word1(p, b'=', Record::RecordEquals)?;
                                chomp_and_check_indent(
                                    p,
                                    Record::RecordSpace,
                                    Record::RecordIndentExpr,
                                )?;
                                let end = specialize(p, Record::RecordExpr, expression)?;
                                check_indent(p, end, Record::RecordIndentEnd)?;
                                fields(p)
                            },
                        ],
                    )
                }],
            )
        },
    )
}

fn fields(p: &mut P<'_>) -> Result<(), Record> {
    loop {
        let more = one_of(
            p,
            Record::RecordEnd,
            &mut [
                &mut |p| {
                    word1(p, b',', Record::RecordEnd)?;
                    chomp_and_check_indent(p, Record::RecordSpace, Record::RecordIndentField)?;
                    field(p).map(|()| true)
                },
                &mut |p| word1(p, b'}', Record::RecordEnd).map(|()| false),
            ],
        )?;
        if !more {
            return Ok(());
        }
    }
}

fn field(p: &mut P<'_>) -> Result<(), Record> {
    lower(p, Record::RecordField)?;
    chomp_and_check_indent(p, Record::RecordSpace, Record::RecordIndentEquals)?;
    word1(p, b'=', Record::RecordEquals)?;
    chomp_and_check_indent(p, Record::RecordSpace, Record::RecordIndentExpr)?;
    let end = specialize(p, Record::RecordExpr, expression)?;
    check_indent(p, end, Record::RecordIndentEnd)
}

// EXPRESSIONS

/// `expression`: an expression, and where it ends before the spaces
/// after it.
pub(super) fn expression(p: &mut P<'_>) -> Result<Cursor, Expr> {
    deep(p, |p| {
        one_of(
            p,
            Expr::Start,
            &mut [&mut let_, &mut if_, &mut case, &mut function, &mut |p| {
                possibly_negative_term(p)?;
                let end = p.cur;
                chomp(p, Expr::Space)?;
                expr_end(p, end)
            }],
        )
    })
}

/// What one more step of `chompExprEnd` read.
enum Step {
    More(Cursor),
    Done(Cursor),
}

/// `chompExprEnd`: arguments and operators after a term ending at `end`.
fn expr_end(p: &mut P<'_>, mut end: Cursor) -> Result<Cursor, Expr> {
    loop {
        let step = one_of_with_fallback(
            p,
            &mut [
                &mut |p| {
                    check_indent(p, end, Expr::Start)?;
                    term(p)?;
                    let new_end = p.cur;
                    chomp(p, Expr::Space)?;
                    Ok(Step::More(new_end))
                },
                &mut |p| {
                    check_indent(p, end, Expr::Start)?;
                    let op_start = p.cur;
                    let op = operator(p, Expr::Start, Expr::OperatorReserved)?;
                    let op_end = p.cur;
                    chomp_and_check_indent(p, Expr::Space, |c| {
                        Expr::IndentOperatorRight(op.clone(), c)
                    })?;
                    let new_start = p.cur;
                    if &*op == "-" && end != op_start && op_end == new_start {
                        term(p)?;
                        let new_end = p.cur;
                        chomp(p, Expr::Space)?;
                        return Ok(Step::More(new_end));
                    }
                    operand(p, &op)
                },
            ],
            Step::Done(end),
        )?;
        match step {
            Step::More(new_end) => end = new_end,
            Step::Done(end) => return Ok(end),
        }
    }
}

/// What may follow an operator: a term, or a final `let`, `case`, `if` or
/// lambda.
fn operand(p: &mut P<'_>, op: &Name) -> Result<Step, Expr> {
    let err = |c| Expr::OperatorRight(op.clone(), c);
    one_of(
        p,
        err,
        &mut [
            &mut |p| {
                possibly_negative_term(p)?;
                let new_end = p.cur;
                chomp(p, Expr::Space)?;
                Ok(Step::More(new_end))
            },
            &mut |p| {
                one_of(p, err, &mut [&mut let_, &mut case, &mut if_, &mut function]).map(Step::Done)
            },
        ],
    )
}

fn possibly_negative_term(p: &mut P<'_>) -> Result<(), Expr> {
    one_of(
        p,
        Expr::Start,
        &mut [
            &mut |p| {
                word1(p, b'-', Expr::Start)?;
                term(p)
            },
            &mut term,
        ],
    )
}

// IF EXPRESSION

fn if_(p: &mut P<'_>) -> Result<Cursor, Expr> {
    in_context(
        p,
        boxed(Expr::If),
        |p| keyword(p, b"if", Expr::Start),
        |p| loop {
            chomp_and_check_indent(p, If::IfSpace, If::IfIndentCondition)?;
            let cond_end = specialize(p, If::IfCondition, expression)?;
            check_indent(p, cond_end, If::IfIndentThen)?;
            keyword(p, b"then", If::IfThen)?;
            chomp_and_check_indent(p, If::IfSpace, If::IfIndentThenBranch)?;
            let then_end = specialize(p, If::IfThenBranch, expression)?;
            check_indent(p, then_end, If::IfIndentElse)?;
            keyword(p, b"else", If::IfElse)?;
            chomp_and_check_indent(p, If::IfSpace, If::IfIndentElseBranch)?;
            let else_end = one_of(
                p,
                If::IfElseBranchStart,
                &mut [
                    &mut |p| keyword(p, b"if", If::IfElseBranchStart).map(|()| None),
                    &mut |p| specialize(p, If::IfElseBranch, expression).map(Some),
                ],
            )?;
            if let Some(end) = else_end {
                return Ok(end);
            }
        },
    )
}

// LAMBDA EXPRESSION

fn function(p: &mut P<'_>) -> Result<Cursor, Expr> {
    in_context(
        p,
        boxed(Expr::Func),
        |p| word1(p, b'\\', Expr::Start),
        |p| {
            chomp_and_check_indent(p, Func::FuncSpace, Func::FuncIndentArg)?;
            specialize(p, Func::FuncArg, pattern::term)?;
            chomp_and_check_indent(p, Func::FuncSpace, Func::FuncIndentArrow)?;
            loop {
                let more = one_of(
                    p,
                    Func::FuncArrow,
                    &mut [
                        &mut |p| {
                            specialize(p, Func::FuncArg, pattern::term)?;
                            chomp_and_check_indent(p, Func::FuncSpace, Func::FuncIndentArrow)?;
                            Ok(true)
                        },
                        &mut |p| word2(p, b'-', b'>', Func::FuncArrow).map(|()| false),
                    ],
                )?;
                if !more {
                    break;
                }
            }
            chomp_and_check_indent(p, Func::FuncSpace, Func::FuncIndentBody)?;
            specialize(p, Func::FuncBody, expression)
        },
    )
}

// CASE EXPRESSIONS

fn case(p: &mut P<'_>) -> Result<Cursor, Expr> {
    in_context(
        p,
        boxed(Expr::Case),
        |p| keyword(p, b"case", Expr::Start),
        |p| {
            chomp_and_check_indent(p, Case::CaseSpace, Case::CaseIndentExpr)?;
            let expr_end = specialize(p, Case::CaseExpr, expression)?;
            check_indent(p, expr_end, Case::CaseIndentOf)?;
            keyword(p, b"of", Case::CaseOf)?;
            chomp_and_check_indent(p, Case::CaseSpace, Case::CaseIndentPattern)?;
            with_indent(p, |p| {
                let mut end = branch(p)?;
                loop {
                    let next = one_of_with_fallback(
                        p,
                        &mut [&mut |p| {
                            check_aligned(p, Case::CasePatternAlignment)?;
                            branch(p).map(Some)
                        }],
                        None,
                    )?;
                    match next {
                        Some(new_end) => end = new_end,
                        None => return Ok(end),
                    }
                }
            })
        },
    )
}

fn branch(p: &mut P<'_>) -> Result<Cursor, Case> {
    let pattern_end = specialize(p, Case::CasePattern, pattern::expression)?;
    check_indent(p, pattern_end, Case::CaseIndentArrow)?;
    word2(p, b'-', b'>', Case::CaseArrow)?;
    chomp_and_check_indent(p, Case::CaseSpace, Case::CaseIndentBranch)?;
    specialize(p, Case::CaseBranch, expression)
}

// LET EXPRESSION

fn let_(p: &mut P<'_>) -> Result<Cursor, Expr> {
    in_context(
        p,
        boxed(Expr::Let),
        |p| keyword(p, b"let", Expr::Start),
        |p| {
            let defs_end = with_backset_indent(p, 3, |p| {
                chomp_and_check_indent(p, Let::LetSpace, Let::LetIndentDef)?;
                with_indent(p, |p| {
                    let mut end = let_def(p)?;
                    loop {
                        let next = one_of_with_fallback(
                            p,
                            &mut [&mut |p| {
                                check_aligned(p, Let::LetDefAlignment)?;
                                let_def(p).map(Some)
                            }],
                            None,
                        )?;
                        match next {
                            Some(new_end) => end = new_end,
                            None => return Ok(end),
                        }
                    }
                })
            })?;
            check_indent(p, defs_end, Let::LetIndentIn)?;
            keyword(p, b"in", Let::LetIn)?;
            chomp_and_check_indent(p, Let::LetSpace, Let::LetIndentBody)?;
            specialize(p, Let::LetBody, expression)
        },
    )
}

fn let_def(p: &mut P<'_>) -> Result<Cursor, Let> {
    one_of(p, Let::LetDefName, &mut [&mut definition, &mut destructure])
}

fn definition(p: &mut P<'_>) -> Result<Cursor, Let> {
    let name = lower(p, Let::LetDefName)?;
    let wrap = |e, c| Let::LetDef(name.clone(), e, c);
    specialize(p, wrap, |p| {
        chomp_and_check_indent(p, Def::DefSpace, Def::DefIndentEquals)?;
        one_of(
            p,
            Def::DefEquals,
            &mut [
                &mut |p| {
                    word1(p, b':', Def::DefEquals)?;
                    chomp_and_check_indent(p, Def::DefSpace, Def::DefIndentType)?;
                    specialize(p, Def::DefType, types::expression)?;
                    check_aligned(p, Def::DefAlignment)?;
                    matching_name(p, &name)?;
                    chomp_and_check_indent(p, Def::DefSpace, Def::DefIndentEquals)?;
                    args_and_body(p)
                },
                &mut args_and_body,
            ],
        )
    })
}

fn args_and_body(p: &mut P<'_>) -> Result<Cursor, Def> {
    loop {
        let body_end = one_of(
            p,
            Def::DefEquals,
            &mut [
                &mut |p| {
                    specialize(p, Def::DefArg, pattern::term)?;
                    chomp_and_check_indent(p, Def::DefSpace, Def::DefIndentEquals)?;
                    Ok(None)
                },
                &mut |p| {
                    word1(p, b'=', Def::DefEquals)?;
                    chomp_and_check_indent(p, Def::DefSpace, Def::DefIndentBody)?;
                    specialize(p, Def::DefBody, expression).map(Some)
                },
            ],
        )?;
        if let Some(end) = body_end {
            return Ok(end);
        }
    }
}

/// `chompMatchingName`: the name a type annotation is for, again.
fn matching_name(p: &mut P<'_>, expected: &Name) -> Result<(), Def> {
    let start = p.cur;
    let name = lower(p, Def::DefNameRepeat)?;
    if name == *expected {
        Ok(())
    } else {
        p.cerr(Def::DefNameMatch(name, start))
    }
}

fn destructure(p: &mut P<'_>) -> Result<Cursor, Let> {
    specialize(p, Let::LetDestruct, |p| {
        specialize(p, Destruct::DestructPattern, pattern::term)?;
        chomp_and_check_indent(p, Destruct::DestructSpace, Destruct::DestructIndentEquals)?;
        word1(p, b'=', Destruct::DestructEquals)?;
        chomp_and_check_indent(p, Destruct::DestructSpace, Destruct::DestructIndentBody)?;
        specialize(p, Destruct::DestructBody, expression)
    })
}
