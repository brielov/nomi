//! `Parse/Type.hs`.

use nomi_base::grow_large;

use super::engine::{P, R};
use super::helpers::{
    Minus, betwixt, cap_var, dot_sep1, lenient_has_type, low_var, record, right_arrow, separated,
    space_prefix, surround_commented,
};
use super::whitespace::with_eol;

fn tvar(p: &mut P<'_>) -> R<()> {
    low_var(p).map(drop)
}

fn tuple(p: &mut P<'_>) -> R<()> {
    p.track_newline(|p| surround_commented(p, '(', ')', &mut |p| with_eol(p, expr)))
        .map(drop)
}

fn constructor0(p: &mut P<'_>) -> R<()> {
    dot_sep1(p, cap_var).map(drop)
}

/// `term`.
pub(super) fn term(p: &mut P<'_>) -> R<()> {
    grow_large(|| {
        p.alts(&mut [
            &mut tuple,
            &mut |p| record(p, lenient_has_type, expr),
            &mut tvar,
            &mut |p| p.track_newline(constructor0),
        ])
    })
}

fn tuple_ctor(p: &mut P<'_>) -> R<()> {
    betwixt(p, '(', ')', |p| p.many1(|p| p.char(','))).map(drop)
}

fn app(p: &mut P<'_>) -> R<()> {
    p.track_newline(|p| {
        p.alt(constructor0, |p| p.attempt(tuple_ctor))?;
        space_prefix(p, Minus::Anywhere, &mut term)
    })
}

/// `expr`.
pub(super) fn expr(p: &mut P<'_>) -> R<()> {
    separated(p, right_arrow, &mut |p| p.alt(app, term)).map(drop)
}

/// `tag`: a variant and its arguments.
pub(super) fn tag(p: &mut P<'_>) -> R<()> {
    cap_var(p)?;
    space_prefix(p, Minus::Anywhere, &mut term)
}
