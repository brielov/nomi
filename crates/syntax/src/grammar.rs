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
use crate::parser::{Layout, Parse, Parser, SyntaxErrorKind};

/// Parses a whole module.
pub fn parse_module(text: &str) -> Parse {
    parse_layouts(text, module::module)
}

/// Parses declarations with no header or imports. elm-format reads an
/// Elm code block in a doc comment this way first.
pub fn parse_declarations(text: &str) -> Parse {
    parse_layouts(text, |p| {
        let m = p.start();
        declarations::declarations(p);
        m.complete(p, K::DeclarationsFragment);
    })
}

/// Parses expressions, each starting a line. elm-format reads an Elm code
/// block in a doc comment this way if it holds no declarations, before
/// trying a whole module.
pub fn parse_expressions(text: &str) -> Parse {
    parse_layouts(text, |p| {
        let m = p.start();
        expressions::expressions(p);
        m.complete(p, K::ExpressionsFragment);
    })
}

/// Parses with [`Layout::Strict`], whose errors and trees suit unfinished
/// code, and if that fails, again with [`Layout::Loose`]: a text that
/// parses cleanly so is one elm-format takes, and its unindented tokens
/// are left for elm's validation.
fn parse_layouts(text: &str, rule: fn(&mut Parser<'_>)) -> Parse {
    let mut p = Parser::new(text);
    rule(&mut p);
    let strict = p.finish();
    if strict.errors().is_empty() {
        return strict;
    }
    let mut p = Parser::with_layout(text, Layout::Loose);
    rule(&mut p);
    let loose = p.finish();
    if loose.errors().is_empty() {
        loose
    } else {
        strict
    }
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
        if p.at_indented_strictly() {
            skip_indented(p, |p| p.at_symbol(kind));
        }
        if !p.at_symbol(kind) {
            return false;
        }
    }
    p.bump_symbol(kind);
    true
}

/// Whether the `n`th token ahead is an operator: upstream's `anyOp`, a
/// symbol or a backticked function, which lexes as one, or a `.` that
/// starts no accessor, which only elm-format takes.
fn at_operator_token(p: &mut Parser<'_>, n: usize) -> bool {
    match p.nth(n) {
        K::Operator | K::Minus | K::ColonColon => true,
        K::Dot => !at_accessor(p, n),
        _ => false,
    }
}

/// Whether the `n`th token ahead starts a `.field` accessor: a `.`
/// touching the lower name after it.
fn at_accessor(p: &mut Parser<'_>, n: usize) -> bool {
    p.nth(n) == K::Dot && p.nth(n + 1) == K::LowerIdent && p.nth_is_joined(n + 1)
}

/// Whether `kind` is an operator that may stand in parentheses: elm takes
/// `..` too (`Parse/Symbol.hs`, `operator`), and elm-format `.`.
fn is_parenthesized_op(kind: K) -> bool {
    matches!(
        kind,
        K::Operator | K::Minus | K::ColonColon | K::DotDot | K::Dot
    )
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

/// The rest of a comma-separated sequence, after its first item: each `,`
/// and the `item` after it, up to and including `close`. Indented junk
/// before a comma or `close` is skipped with one error; anything else
/// ends the sequence, `close` missing. Returns whether a comma came.
fn sequence_tail(p: &mut Parser<'_>, close: K, item: impl Fn(&mut Parser<'_>)) -> bool {
    let mut comma = false;
    loop {
        if p.at(close) {
            p.bump();
            break;
        }
        if p.at(K::Comma) {
            p.bump();
            comma = true;
            item(p);
        } else if p.at_indented_strictly() {
            skip_junk(p, SyntaxErrorKind::Expected(close), &[K::Comma, close]);
        } else {
            p.error(SyntaxErrorKind::Expected(close));
            break;
        }
    }
    comma
}

/// Whether the tokens from the `n`th on are commas, at least one, then
/// `)`: the inside of `(,)`, `(,,)` and so on.
fn at_tuple_constructor(p: &mut Parser<'_>, n: usize) -> bool {
    let mut scan = p.scan(n);
    if scan.nth(0) != K::Comma {
        return false;
    }
    while scan.nth(0) == K::Comma {
        scan.skip(1);
    }
    scan.nth(0) == K::RParen
}

/// The commas and the `)` of a tuple constructor, after its `(`.
fn tuple_constructor_tail(p: &mut Parser<'_>) {
    while p.at(K::Comma) {
        p.bump();
    }
    p.bump();
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
    if !p.at_indented_strictly() || stops.contains(&p.current()) {
        return;
    }
    p.error(error);
    skip_indented(p, |p| stops.contains(&p.current()));
}

/// Wraps the continuation tokens before the first where `stop` holds in
/// one [`K::Error`] node, reporting nothing. The current token is one.
fn skip_indented(p: &mut Parser<'_>, stop: impl Fn(&mut Parser<'_>) -> bool) {
    let m = p.start();
    while p.at_indented_strictly() && !stop(p) {
        p.bump();
    }
    m.complete(p, K::Error);
}

#[cfg(test)]
mod testing {
    use super::K;
    use crate::parser::{Parse, Parser, SyntaxErrorKind};

    /// A rule parsed where the fixtures' fragments can't reach it: at the
    /// top level, after `prefix` tokens at the margin, with what the rule
    /// leaves swept into an [`K::Error`] node.
    pub(super) struct TopLevel {
        pub(super) prefix: usize,
        pub(super) rule: fn(&mut Parser<'_>),
        pub(super) root: K,
    }

    impl TopLevel {
        pub(super) fn parse(&self, text: &str) -> Parse {
            let mut p = Parser::new(text);
            let m = p.start();
            p.anchor();
            for _ in 0..self.prefix {
                p.bump();
            }
            (self.rule)(&mut p);
            if !p.at(K::Eof) {
                let rest = p.start();
                p.with_min_column(0, |p| {
                    while !p.at(K::Eof) {
                        p.bump();
                    }
                });
                rest.complete(&mut p, K::Error);
            }
            m.complete(&mut p, self.root);
            p.finish()
        }

        /// The tree, then the errors elm reports, a line each.
        pub(super) fn render(&self, text: &str) -> String {
            use std::fmt::Write;
            let parse = self.parse(text);
            let mut out = parse.debug_tree();
            for error in parse.elm_errors() {
                let _ = writeln!(out, "error {:?}: {:?}", error.range, error.kind);
            }
            out
        }

        #[track_caller]
        pub(super) fn check(&self, text: &str, expected: &str) {
            assert_eq!(self.render(text).trim(), expected.trim());
        }

        /// The errors elm reports, in source order.
        pub(super) fn errors(&self, text: &str) -> Vec<SyntaxErrorKind> {
            (self.parse(text).elm_errors().iter())
                .map(|e| e.kind)
                .collect()
        }
    }
}
