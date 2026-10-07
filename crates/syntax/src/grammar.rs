//! The Elm grammar, as rules over [`Parser`]. It accepts the union of what
//! elm and elm-format accept, legacy syntax included; the validation pass
//! flags what a given tool rejects. `elm.ungram` describes the trees.

mod declarations;
mod definitions;
mod expressions;
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

/// Parses declarations with no header or imports. elm-format reads an
/// Elm code block in a doc comment this way first.
pub fn parse_declarations(text: &str) -> Parse {
    let mut p = Parser::new(text);
    let m = p.start();
    declarations::declarations(&mut p);
    m.complete(&mut p, K::DeclarationsFragment);
    p.finish()
}

/// Parses expressions, each starting a line. elm-format reads an Elm code
/// block in a doc comment this way if it holds no declarations, before
/// trying a whole module.
pub fn parse_expressions(text: &str) -> Parse {
    let mut p = Parser::new(text);
    let m = p.start();
    expressions::expressions(&mut p);
    m.complete(&mut p, K::ExpressionsFragment);
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

/// Parses an expression on its own. Every line may start at column 0.
pub fn parse_expr(text: &str) -> Parse {
    parse_fragment(text, K::ExprFragment, expressions::expr)
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

/// Consumes `kind`, splitting it off an operator it starts, and reports it
/// if it isn't indented; or reports that it is missing.
fn expect_indented(p: &mut Parser<'_>, kind: K) -> bool {
    if !p.at_symbol(kind) {
        p.error(SyntaxErrorKind::Expected(kind));
        return false;
    }
    p.check_indented();
    p.bump_symbol(kind);
    true
}

/// Consumes `kind`, the keyword or symbol that starts the next part of a
/// construct, such as `then`. Anything indented before it is skipped as
/// one error. Returns false, having reported it, if the construct ends
/// first.
fn expect_part(p: &mut Parser<'_>, kind: K) -> bool {
    if !p.at_symbol(kind) {
        p.error(SyntaxErrorKind::Expected(kind));
        if p.at_indented() {
            let m = p.start();
            while p.at_indented() && !p.at_symbol(kind) {
                p.bump();
            }
            m.complete(p, K::Error);
        }
        if !p.at_symbol(kind) {
            return false;
        }
    }
    p.check_indented();
    p.bump_symbol(kind);
    true
}

/// The operator of `(+)`, the current token, after its `(`. Neither
/// parenthesis may stand apart from it.
fn parenthesized_op(p: &mut Parser<'_>) {
    if !p.nth_is_joined(0) {
        p.error(SyntaxErrorKind::SpacedOperator);
    }
    let op = p.start();
    p.bump();
    op.complete(p, K::Op);
    if p.at(K::RParen) && !p.nth_is_joined(0) {
        p.error(SyntaxErrorKind::SpacedOperator);
    }
}

/// Whether the current token lines up at `column` with the items of a
/// block and, by `starts`, starts another. It is then anchored: it counts
/// as indented, though the rest of the item must be further right.
fn at_aligned(p: &mut Parser<'_>, column: u32, starts: fn(&mut Parser<'_>) -> bool) -> bool {
    if p.at(K::Eof) || p.current_column() != column {
        return false;
    }
    p.anchor();
    let at = starts(p);
    if !at {
        p.unanchor();
    }
    at
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
