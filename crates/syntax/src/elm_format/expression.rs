//! `Parse/Expression.hs` and `Parse/Binop.hs`.

use super::engine::{P, R, deep};
use super::helpers::{
    accessible, any_op, betwixt, comma, comma_sep1, commented, constrained_space_prefix, equals,
    has_type, lenient_equals, low_var, pair, reserved, right_arrow, sectioned_group, shader,
    space_prefix, surround, sym_op, var,
};
use super::literal::literal;
use super::whitespace::{padded, whitespace};
use super::{pattern, types};

// TERMS

fn accessor(p: &mut P<'_>) -> R<()> {
    p.attempt(|p| {
        p.string(".")?;
        low_var(p)
    })
    .map(drop)
}

fn negative(p: &mut P<'_>) -> R<()> {
    p.attempt(|p| {
        p.char('-')?;
        p.not_followed_by(|p| p.alt(|p| p.char('.'), |p| p.char('-')))?;
        term(p)
    })
}

fn list_term(p: &mut P<'_>) -> R<()> {
    p.alts(&mut [
        &mut shader,
        &mut |p| {
            p.attempt(|p| {
                surround(p, '[', ']', |p| {
                    expr(p)?;
                    padded(p, |p| p.string(".."))?;
                    expr(p)
                })
            })
        },
        &mut |p| {
            betwixt(p, '[', ']', |p| {
                p.track_newline(|p| sectioned_group(p, &mut expr))
            })
            .map(drop)
        },
    ])
}

fn parens_term(p: &mut P<'_>) -> R<()> {
    p.choice(&mut [
        &mut |p| p.attempt(|p| betwixt(p, '(', ')', any_op)),
        &mut |p| p.attempt(|p| betwixt(p, '(', ')', |p| p.many1(comma)).map(drop)),
        &mut |p| {
            surround(p, '(', ')', |p| {
                p.alt(|p| comma_sep1(p, &mut expr), |_| Ok(()))
            })
        },
    ])
}

fn record_term(p: &mut P<'_>) -> R<()> {
    betwixt(p, '{', '}', |p| {
        p.track_newline(|p| {
            p.option(|p| {
                p.attempt(|p| {
                    commented(p, low_var)?;
                    p.string("|")
                })
            })?;
            sectioned_group(p, &mut |p| pair(p, low_var, lenient_equals, expr))
        })
    })
    .map(drop)
}

/// `term`.
pub(super) fn term(p: &mut P<'_>) -> R<()> {
    deep(p, |p| {
        p.alt(
            |p| p.choice(&mut [&mut literal, &mut list_term, &mut accessor, &mut negative]),
            |p| {
                accessible(p, |p| {
                    p.alts(&mut [&mut var, &mut parens_term, &mut record_term])
                })
            },
        )
    })
}

fn app_expr(p: &mut P<'_>) -> R<()> {
    p.track_newline(term)?;
    constrained_space_prefix(p, &mut term)
}

// EXPRESSIONS

/// `expr`.
pub(super) fn expr(p: &mut P<'_>) -> R<()> {
    deep(p, |p| {
        p.alts(&mut [&mut keyword_expr, &mut lambda_expr, &mut binary_expr])
    })
}

fn keyword_expr(p: &mut P<'_>) -> R<()> {
    p.choice(&mut [&mut let_expr, &mut case_expr, &mut if_expr])
}

/// `binops`. Upstream's `nextOps` recurses into itself once an operand
/// is an application: here it loops.
fn binary_expr(p: &mut P<'_>) -> R<()> {
    p.track_newline(|p| {
        app_expr(p)?;
        loop {
            let more = p.option(|p| {
                p.commit_if(
                    |p| {
                        whitespace(p)?;
                        any_op(p)
                    },
                    |p| {
                        whitespace(p)?;
                        any_op(p)?;
                        whitespace(p)?;
                        p.alt(
                            |p| p.attempt(app_expr).map(|()| true),
                            |p| p.alt(keyword_expr, lambda_expr).map(|()| false),
                        )
                    },
                )
            })?;
            if more != Some(true) {
                return Ok(());
            }
        }
    })
    .map(drop)
}

fn if_expr(p: &mut P<'_>) -> R<()> {
    let else_keyword = |p: &mut P<'_>| {
        reserved(p, "else")?;
        whitespace(p).map(drop)
    };
    if_clause(p)?;
    p.many(|p| {
        p.attempt(|p| {
            else_keyword(p)?;
            if_clause(p)
        })
    })?;
    else_keyword(p)?;
    expr(p)
}

fn if_clause(p: &mut P<'_>) -> R<()> {
    p.attempt(|p| reserved(p, "if"))?;
    whitespace(p)?;
    expr(p)?;
    padded(p, |p| reserved(p, "then"))?;
    expr(p)?;
    whitespace(p).map(drop)
}

fn lambda_expr(p: &mut P<'_>) -> R<()> {
    p.track_newline(|p| {
        p.alt(|p| p.char('\\'), |p| p.char('\u{03BB}'))?;
        space_prefix(p, &mut pattern::term)?;
        padded(p, right_arrow)?;
        expr(p)
    })
    .map(drop)
}

fn case_expr(p: &mut P<'_>) -> R<()> {
    let case = |p: &mut P<'_>| {
        p.attempt(|p| {
            whitespace(p)?;
            p.check_indent()?;
            pattern::expr(p)?;
            padded(p, right_arrow)
        })?;
        expr(p)
    };
    p.attempt(|p| reserved(p, "case"))?;
    p.track_newline(|p| padded(p, expr))?;
    p.alt(|p| reserved(p, "of"), |p| reserved(p, "then"))?;
    whitespace(p)?;
    p.with_pos(|p| {
        case(p)?;
        p.many(case).map(drop)
    })
}

fn let_expr(p: &mut P<'_>) -> R<()> {
    p.attempt(|p| reserved(p, "let"))?;
    whitespace(p)?;
    // `block`.
    p.with_pos(|p| {
        p.many1(|p| {
            p.check_indent()?;
            p.alt(type_annotation, definition)?;
            whitespace(p)
        })
    })?;
    reserved(p, "in")?;
    whitespace(p)?;
    expr(p)
}

// DECLARATIONS

/// `typeAnnotation`.
pub(super) fn type_annotation(p: &mut P<'_>) -> R<()> {
    p.attempt(|p| {
        p.alt(
            |p| low_var(p).map(drop),
            |p| betwixt(p, '(', ')', sym_op).map(drop),
        )?;
        padded(p, has_type)
    })?;
    types::expr(p)
}

/// `definition`.
pub(super) fn definition(p: &mut P<'_>) -> R<()> {
    p.with_pos(|p| {
        def_start(p)?;
        padded(p, equals)?;
        expr(p)
    })
}

/// `defStart`: a function's name and arguments, or a pattern.
fn def_start(p: &mut P<'_>) -> R<()> {
    p.choice(&mut [
        &mut |p| {
            if p.attempt(pattern::term)? {
                space_prefix(p, &mut pattern::term)?;
            }
            Ok(())
        },
        &mut |p| {
            betwixt(p, '(', ')', sym_op)?;
            space_prefix(p, &mut pattern::term)
        },
    ])
}
