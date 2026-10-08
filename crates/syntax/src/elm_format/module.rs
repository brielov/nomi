//! `Parse/Module.hs` and `Parse/Declaration.hs`.

use nomi_base::ghc;

use super::engine::{Dyn, P, R};
use super::helpers::{
    any_op, betwixt, cap_var, comma_sep1, commented_keyword, dot_sep1, equals, has_type, key_value,
    low_var, reserved, space_prefix, spacey_sep_by1, surround, sym_op, vertical_bar,
};
use super::whitespace::{doc_comment, forced_ws, fresh_line, padded, spaces, whitespace};
use super::{expression, types};

/// `elmModule`.
pub(super) fn module(p: &mut P<'_>) -> R<()> {
    p.option(fresh_line)?;
    module_decl(p)?;
    p.option(fresh_line)?;
    p.option(|p| {
        doc_comment(p)?;
        fresh_line(p)
    })?;
    p.many(|p| {
        import(p)?;
        fresh_line(p)
    })?;
    top_level(p)?;
    p.option(fresh_line)?;
    p.option(spaces)?;
    p.eof()
}

/// `topLevel declaration`.
fn top_level(p: &mut P<'_>) -> R<()> {
    p.option(top_level_structure)?;
    p.many(|p| {
        p.commit_if(
            |p| {
                fresh_line(p)?;
                p.alt(|p| p.satisfy(ghc::is_alpha), |p| p.satisfy(|c| c == '_'))
                    .map(drop)
            },
            |p| {
                fresh_line(p)?;
                top_level_structure(p)
            },
        )
    })?;
    Ok(())
}

fn top_level_structure(p: &mut P<'_>) -> R<()> {
    p.choice(&mut [&mut doc_comment, &mut declaration])
}

// HEADER

fn module_decl(p: &mut P<'_>) -> R<()> {
    p.choice(&mut [
        &mut |p| p.attempt(module_decl_0_16),
        &mut module_decl_0_17,
        &mut |_| Ok(()),
    ])
}

fn module_decl_0_16(p: &mut P<'_>) -> R<()> {
    p.attempt(|p| reserved(p, "module"))?;
    whitespace(p)?;
    dot_sep1(p, cap_var)?;
    whitespace(p)?;
    p.option(|p| listing(p, &mut detailed_listing))?;
    whitespace(p)?;
    reserved(p, "where")
}

fn module_decl_0_17(p: &mut P<'_>) -> R<()> {
    p.attempt(|p| {
        p.choice(&mut [
            &mut |p| {
                reserved(p, "port")?;
                whitespace(p).map(drop)
            },
            &mut |p| {
                reserved(p, "effect")?;
                whitespace(p).map(drop)
            },
            &mut |_| Ok(()),
        ])?;
        reserved(p, "module")
    })?;
    whitespace(p)?;
    dot_sep1(p, cap_var)?;
    p.option(|p| {
        commented_keyword(p, "where", |p| {
            surround(p, '{', '}', |p| {
                comma_sep1(p, &mut |p| key_value(p, equals, low_var, cap_var))
            })
        })
    })?;
    p.option(|p| {
        p.alt(
            |p| commented_keyword(p, "exposing", |p| listing(p, &mut detailed_listing)),
            |p| p.attempt(listing_without_exposing),
        )
    })?;
    Ok(())
}

fn listing_without_exposing(p: &mut P<'_>) -> R<()> {
    whitespace(p)?;
    listing(p, &mut detailed_listing)
}

// IMPORTS

fn import(p: &mut P<'_>) -> R<()> {
    p.attempt(|p| reserved(p, "import"))?;
    whitespace(p)?;
    dot_sep1(p, cap_var)?;
    p.option(|p| {
        p.attempt(|p| {
            whitespace(p)?;
            reserved(p, "as")
        })?;
        whitespace(p)?;
        cap_var(p)
    })?;
    p.option(|p| p.alt(exposing, |p| p.attempt(listing_without_exposing)))?;
    Ok(())
}

fn exposing(p: &mut P<'_>) -> R<()> {
    p.attempt(|p| {
        whitespace(p)?;
        reserved(p, "exposing")
    })?;
    whitespace(p)?;
    p.choice(&mut [&mut |p| listing(p, &mut detailed_listing), &mut |p| {
        p.choice(&mut [&mut |p| p.string(".."), &mut detailed_listing])
    }])
}

/// `listing`: `(..)`, or `explicit` in parentheses.
fn listing<'s>(p: &mut P<'s>, explicit: Dyn<'_, 's, ()>) -> R<()> {
    p.attempt(|p| p.char('('))?;
    p.track_newline(|p| {
        whitespace(p)?;
        p.choice(&mut [&mut |p| p.string(".."), &mut *explicit])?;
        whitespace(p)
    })?;
    p.char(')')
}

fn detailed_listing(p: &mut P<'_>) -> R<()> {
    comma_sep1(p, &mut value)
}

fn value(p: &mut P<'_>) -> R<()> {
    p.alts(&mut [
        &mut |p| low_var(p).map(drop),
        &mut |p| betwixt(p, '(', ')', sym_op).map(drop),
        &mut |p| {
            cap_var(p)?;
            p.option(|p| {
                p.attempt(|p| {
                    whitespace(p)?;
                    listing(p, &mut |p| comma_sep1(p, &mut |p| cap_var(p).map(drop)))
                })
            })?;
            Ok(())
        },
    ])
}

// DECLARATIONS

/// `declaration`.
fn declaration(p: &mut P<'_>) -> R<()> {
    p.alts(&mut [
        &mut type_decl,
        &mut |p| p.attempt(infix_decl),
        &mut port,
        &mut |p| p.alt(expression::type_annotation, expression::definition),
    ])
}

fn type_decl(p: &mut P<'_>) -> R<()> {
    p.attempt(|p| reserved(p, "type"))?;
    forced_ws(p)?;
    let alias = p.option(|p| {
        p.string("alias")?;
        forced_ws(p)
    })?;
    cap_var(p)?;
    space_prefix(p, &mut low_var)?;
    padded(p, equals)?;
    if alias.is_some() {
        types::expr(p)
    } else {
        spacey_sep_by1(p, vertical_bar, &mut types::tag)
    }
}

fn infix_decl(p: &mut P<'_>) -> R<()> {
    p.choice(&mut [&mut |p| p.attempt(infix_0_16), &mut infix_0_19])
}

fn infix_0_16(p: &mut P<'_>) -> R<()> {
    p.choice(&mut [
        &mut |p| p.attempt(|p| reserved(p, "infixl")),
        &mut |p| p.attempt(|p| reserved(p, "infixr")),
        &mut |p| p.attempt(|p| reserved(p, "infix")),
    ])?;
    forced_ws(p)?;
    p.digit()?;
    forced_ws(p)?;
    any_op(p)
}

fn infix_0_19(p: &mut P<'_>) -> R<()> {
    p.attempt(|p| reserved(p, "infix"))?;
    whitespace(p)?;
    p.choice(&mut [
        &mut |p| p.string("right"),
        &mut |p| p.string("non"),
        &mut |p| p.string("left"),
    ])?;
    whitespace(p)?;
    p.digit()?;
    whitespace(p)?;
    betwixt(p, '(', ')', sym_op)?;
    whitespace(p)?;
    equals(p)?;
    whitespace(p)?;
    low_var(p).map(drop)
}

fn port(p: &mut P<'_>) -> R<()> {
    p.attempt(|p| reserved(p, "port"))?;
    whitespace(p)?;
    low_var(p)?;
    whitespace(p)?;
    p.choice(&mut [
        &mut |p| {
            p.attempt(has_type)?;
            whitespace(p)?;
            types::expr(p)
        },
        &mut |p| {
            p.attempt(equals)?;
            whitespace(p)?;
            expression::expr(p)
        },
    ])
}
