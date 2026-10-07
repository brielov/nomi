//! The Elm grammar, as rules over [`Parser`]. It accepts the union of what
//! elm and elm-format accept, legacy syntax included; the validation pass
//! flags what a given tool rejects. `elm.ungram` describes the trees.

mod module;

use crate::SyntaxKind as K;
use crate::parser::{Parse, Parser, SyntaxErrorKind};

/// Parses a whole module.
pub fn parse_module(text: &str) -> Parse {
    let mut p = Parser::new(text);
    module::module(&mut p);
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

/// Wraps tokens that continue a finished construct on its lines in one
/// [`K::Error`] node, with one error.
fn skip_indented(p: &mut Parser<'_>) {
    if !p.at_indented() {
        return;
    }
    p.error(SyntaxErrorKind::Unexpected);
    let m = p.start();
    while p.at_indented() {
        p.bump();
    }
    m.complete(p, K::Error);
}
