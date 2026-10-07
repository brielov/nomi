//! The Elm grammar, as rules over [`Parser`]. It accepts the union of what
//! elm and elm-format accept, legacy syntax included; the validation pass
//! flags what a given tool rejects. `elm.ungram` describes the trees.

mod module;
mod patterns;
mod types;

use crate::SyntaxKind as K;
use crate::parser::{Parse, Parser, SyntaxErrorKind};

/// Parses a whole module.
pub fn parse_module(text: &str) -> Parse {
    let mut p = Parser::new(text);
    module::module(&mut p);
    p.finish()
}

/// Parses a type on its own, such as one from a package's `docs.json`.
/// Every line may start at column 0.
pub fn parse_type(text: &str) -> Parse {
    parse_fragment(text, K::TypeFragment, types::type_)
}

/// Parses a pattern on its own. Every line may start at column 0.
pub fn parse_pattern(text: &str) -> Parse {
    parse_fragment(text, K::PatternFragment, patterns::pattern)
}

fn parse_fragment(text: &str, root: K, rule: fn(&mut Parser<'_>)) -> Parse {
    let mut p = Parser::new(text);
    let m = p.start();
    p.with_min_column(0, |p| {
        rule(p);
        skip_junk(p, SyntaxErrorKind::Unexpected, &[]);
    });
    m.complete(&mut p, root);
    p.finish()
}

/// Consumes `kind`, reporting it if it isn't indented, or reports that it
/// is missing.
fn expect_indented(p: &mut Parser<'_>, kind: K) -> bool {
    if !p.at(kind) {
        p.error(SyntaxErrorKind::Expected(kind));
        return false;
    }
    p.check_indented();
    p.bump();
    true
}

/// A name with an optional qualifier: `Maybe`, `Maybe.Maybe`, or
/// `List.map` where `last` takes lower identifiers. The current token is
/// the first name, and each dot must touch the names around it.
fn qualified_name(p: &mut Parser<'_>, last: &[K]) {
    let q = p.start();
    let mut qualified = false;
    while p.at(K::UpperIdent)
        && p.nth(1) == K::Dot
        && p.nth_is_joined(1)
        && last.contains(&p.nth(2))
        && p.nth_is_joined(2)
    {
        p.bump();
        p.bump();
        qualified = true;
    }
    if qualified {
        q.complete(p, K::Qualifier);
    } else {
        q.abandon(p);
    }
    p.bump();
}

/// `Foo.Bar.Baz`, a continuation. Each dot and name must touch the one
/// before.
fn module_name(p: &mut Parser<'_>) {
    let m = p.start();
    if p.at(K::UpperIdent) {
        p.check_indented();
        p.bump();
        while p.at(K::Dot) && p.nth_is_joined(0) && p.nth(1) == K::UpperIdent && p.nth_is_joined(1)
        {
            p.bump();
            p.bump();
        }
    } else {
        p.error(SyntaxErrorKind::Expected(K::ModuleName));
    }
    m.complete(p, K::ModuleName);
}

/// Wraps the continuation tokens before the next of `stops` in one
/// [`K::Error`] node, with one `error`.
fn skip_junk(p: &mut Parser<'_>, error: SyntaxErrorKind, stops: &[K]) {
    if !p.at_indented() || stops.contains(&p.current()) {
        return;
    }
    p.error(error);
    let m = p.start();
    while p.at_indented() && !stops.contains(&p.current()) {
        p.bump();
    }
    m.complete(p, K::Error);
}
