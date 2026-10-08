//! `Parse/Type.hs`.

use super::engine::{P, R, deep};
use super::helpers::{
    betwixt, cap_var, commented, dot_sep1, lenient_has_type, low_var, pair, right_arrow,
    sectioned_group, separated, space_prefix, surround__,
};
use super::whitespace::with_eol;

fn tvar(p: &mut P<'_>) -> R<()> {
    low_var(p).map(drop)
}

fn tuple(p: &mut P<'_>) -> R<()> {
    p.track_newline(|p| surround__(p, '(', ')', &mut |p| with_eol(p, expr)))
        .map(drop)
}

fn record(p: &mut P<'_>) -> R<()> {
    betwixt(p, '{', '}', |p| {
        p.track_newline(|p| {
            p.option(|p| {
                p.attempt(|p| {
                    commented(p, low_var)?;
                    p.string("|")
                })
            })?;
            sectioned_group(p, &mut |p| pair(p, low_var, lenient_has_type, expr))
        })
    })
    .map(drop)
}

fn constructor0(p: &mut P<'_>) -> R<()> {
    dot_sep1(p, cap_var).map(drop)
}

/// `term`.
pub(super) fn term(p: &mut P<'_>) -> R<()> {
    deep(p, |p| {
        p.alts(&mut [&mut tuple, &mut record, &mut tvar, &mut |p| {
            p.track_newline(constructor0).map(drop)
        }])
    })
}

fn tuple_ctor(p: &mut P<'_>) -> R<()> {
    betwixt(p, '(', ')', |p| p.many1(|p| p.char(','))).map(drop)
}

fn app(p: &mut P<'_>) -> R<()> {
    p.track_newline(|p| {
        p.alt(constructor0, |p| p.attempt(tuple_ctor))?;
        space_prefix(p, &mut term)
    })
    .map(drop)
}

/// `expr`.
pub(super) fn expr(p: &mut P<'_>) -> R<()> {
    separated(p, right_arrow, &mut |p| p.alt(app, term)).map(drop)
}

/// `tag`: a variant and its arguments.
pub(super) fn tag(p: &mut P<'_>) -> R<()> {
    cap_var(p)?;
    space_prefix(p, &mut term)
}
