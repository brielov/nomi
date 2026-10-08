//! What formatting must keep for every module it accepts: the fixtures,
//! the property tests, the fuzz target and the corpus check these.

use std::collections::BTreeSet;

use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxError, SyntaxKind, SyntaxNode, SyntaxToken, Tool, parse_module, validate};

use crate::format;
use crate::lower::{literal_token, negative};
use crate::tree::LiteralValue;

/// A property of formatting that every module it accepts must keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invariant {
    /// The output parses as elm-format reads modules.
    Parses,
    /// Every comment survives, though moved and respaced, and every doc
    /// comment, though its Markdown is reprinted.
    KeepsComments,
    /// The header and imports still import the same modules.
    KeepsImports,
    /// The declarations' tokens are kept, but for parentheses and the
    /// spelling of literals.
    KeepsTokens,
    /// The output formats to itself.
    Idempotent,
}

impl Invariant {
    pub const ALL: [Invariant; 5] = [
        Invariant::Parses,
        Invariant::KeepsComments,
        Invariant::KeepsImports,
        Invariant::KeepsTokens,
        Invariant::Idempotent,
    ];

    /// The name tables use.
    pub fn name(self) -> &'static str {
        match self {
            Invariant::Parses => "parses",
            Invariant::KeepsComments => "keeps-comments",
            Invariant::KeepsImports => "keeps-imports",
            Invariant::KeepsTokens => "keeps-tokens",
            Invariant::Idempotent => "idempotent",
        }
    }
}

/// Each invariant formatting `text` breaks, in the order of
/// [`Invariant::ALL`], and how; none where it doesn't format.
pub fn broken_invariants(text: &str) -> Vec<(Invariant, String)> {
    let mut broken = Vec::new();
    let Some(output) = format(text) else {
        return broken;
    };
    let parse = parse_module(&output);
    let errors = [parse.errors(), &validate(&parse, Tool::ElmFormat)].concat();
    if !errors.is_empty() {
        broken.push((Invariant::Parses, format!("{errors:?}")));
    }
    let before = Summary::of(&parse_module(text).syntax(), &[]);
    let after = Summary::of(&parse.syntax(), &errors);
    if before.comments != after.comments || before.doc_comments != after.doc_comments {
        let lost: Vec<_> = before
            .comments
            .iter()
            .filter(|c| !after.comments.contains(c))
            .collect();
        let gained: Vec<_> = after
            .comments
            .iter()
            .filter(|c| !before.comments.contains(c))
            .collect();
        let docs = (before.doc_comments, after.doc_comments);
        broken.push((
            Invariant::KeepsComments,
            format!("lost {lost:?}, gained {gained:?}, doc comments {docs:?}"),
        ));
    }
    if before.imports != after.imports {
        let imports = (&before.imports, &after.imports);
        broken.push((Invariant::KeepsImports, format!("{imports:?}")));
    }
    if let Some(at) = (0..before.body.len().max(after.body.len()))
        .find(|&i| before.body.get(i) != after.body.get(i))
    {
        let near = |body: &[Token]| body[at.saturating_sub(3)..(at + 3).min(body.len())].to_vec();
        let (before, after) = (near(&before.body), near(&after.body));
        broken.push((
            Invariant::KeepsTokens,
            format!("{before:?} became {after:?}"),
        ));
    }
    match format(&output) {
        Some(again) if again == output => {}
        again => {
            let line = again
                .as_deref()
                .and_then(|again| output.lines().zip(again.lines()).position(|(a, b)| a != b))
                .map_or(0, |i| i + 1);
            broken.push((
                Invariant::Idempotent,
                format!("formatting again changes line {line}"),
            ));
        }
    }
    broken
}

/// The invariants elm-format 0.8.8 breaks for some modules like `text`,
/// in the order of [`Invariant::ALL`],
/// as nomi does, by what `text` holds:
///
/// - comments: comments in the module header and exposing lists vanish,
///   some there leave an unparseable exposing list, and comments and doc
///   comments' Markdown can move or respace on a second run;
/// - `\x` escapes, which elm-format reads as Elm 0.18 did and prints as
///   other code points, some that don't exist;
/// - a float too large for a double, which elm-format prints as
///   `Infinity`, a name Elm doesn't define;
/// - a call spanning lines of a parenthesized call or of a range, as in
///   `(f x) y` or `[a..b] y` then an argument below, whose function's
///   arguments join the first line and move below it on a second run;
/// - an Elm 0.18 infix declaration spanning lines for a comment in it,
///   which elm-format prints as a request to report it;
/// - a module without a header, declaring a type alias and a custom
///   type of one name, which the exposing list elm-format writes names
///   twice and a second run once;
/// - a doc comment holding a comment, whose nesting reprinting its
///   Markdown can undo, so that it ends early or swallows code;
/// - a `{--` comment whose text starts with `}` after its spaces, which
///   elm-format prints as `{--}` and so ends early.
///
/// The fixtures' and the corpus's `invariants.tsv` record the modules.
pub fn upstream_breaks(text: &str) -> Vec<Invariant> {
    let mut breaks = Vec::new();
    let root = parse_module(text).syntax();
    let tokens = || {
        root.descendants_with_tokens()
            .filter_map(|e| e.into_token())
    };
    if tokens().any(|t| {
        matches!(
            t.kind(),
            SyntaxKind::LineComment | SyntaxKind::BlockComment | SyntaxKind::DocComment
        )
    }) {
        breaks.extend([
            Invariant::Parses,
            Invariant::KeepsComments,
            Invariant::Idempotent,
        ]);
    }
    if tokens().any(|t| {
        matches!(
            t.kind(),
            SyntaxKind::Char | SyntaxKind::String | SyntaxKind::MultilineString
        ) && t.text().contains("\\x")
    }) {
        breaks.extend([
            Invariant::Parses,
            Invariant::KeepsTokens,
            Invariant::Idempotent,
        ]);
    }
    if tokens().any(|t| {
        t.kind() == SyntaxKind::Float && t.text().parse::<f64>().is_ok_and(f64::is_infinite)
    }) {
        breaks.push(Invariant::KeepsTokens);
    }
    if root.descendants().any(|call| {
        call.kind() == SyntaxKind::CallExpr
            && call.text().to_string().trim().contains('\n')
            && call.first_child().is_some_and(|function| {
                function.kind() == SyntaxKind::RangeExpr
                    || function.kind() == SyntaxKind::ParenExpr
                        && function.children().any(|inner| {
                            matches!(inner.kind(), SyntaxKind::CallExpr | SyntaxKind::RangeExpr)
                        })
            })
    }) {
        breaks.push(Invariant::Idempotent);
    }
    if root
        .descendants()
        .filter(|decl| decl.kind() == SyntaxKind::LegacyInfixDecl)
        .flat_map(|decl| {
            decl.descendants_with_tokens()
                .filter_map(|e| e.into_token())
        })
        .any(|t| {
            t.kind() == SyntaxKind::LineComment
                || t.kind() == SyntaxKind::BlockComment && t.text().contains('\n')
        })
    {
        breaks.push(Invariant::KeepsTokens);
    }
    let type_name = |kind| {
        root.children()
            .filter(move |decl| decl.kind() == kind)
            .filter_map(|decl| {
                decl.children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .find(|t| t.kind() == SyntaxKind::UpperIdent)
                    .map(|name| name.text().to_owned())
            })
            .collect::<BTreeSet<_>>()
    };
    let headless = !root
        .children()
        .any(|n| n.kind() == SyntaxKind::ModuleHeader);
    if headless
        && !type_name(SyntaxKind::TypeAliasDecl).is_disjoint(&type_name(SyntaxKind::CustomTypeDecl))
    {
        breaks.push(Invariant::Idempotent);
    }
    if tokens().any(|t| t.kind() == SyntaxKind::DocComment && t.text()[3..].contains("{-")) {
        breaks.extend(Invariant::ALL);
    }
    if tokens().any(|t| {
        t.kind() == SyntaxKind::BlockComment
            && t.text() != "{--}"
            && t.text()
                .strip_prefix("{--")
                .is_some_and(|body| body.trim_start_matches(' ').starts_with('}'))
    }) {
        breaks.extend(Invariant::ALL);
    }
    Invariant::ALL
        .into_iter()
        .filter(|i| breaks.contains(i))
        .collect()
}

/// Formats `text` and panics if formatting breaks an invariant
/// elm-format keeps for modules like it ([`upstream_breaks`]).
pub fn check_invariants(text: &str) {
    let allowed = upstream_breaks(text);
    for (invariant, how) in broken_invariants(text) {
        assert!(
            allowed.contains(&invariant),
            "formatting breaks `{}`: {how}",
            invariant.name()
        );
    }
}

/// What of a module formatting keeps.
struct Summary {
    /// Each comment's text but whitespace, sorted: formatting moves
    /// comments, reindents block comments' lines, and pads `{-x-}`.
    comments: Vec<String>,
    doc_comments: usize,
    imports: BTreeSet<String>,
    /// The declarations' tokens, but parentheses, which formatting adds
    /// and removes.
    body: Vec<Token>,
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Literal(LiteralValue),
    Other(SyntaxKind, String),
}

impl Summary {
    /// What `root` holds, where `errors` are its parse's: a literal one
    /// covers is compared by its text, as it has no value.
    fn of(root: &SyntaxNode, errors: &[SyntaxError]) -> Self {
        let valid = |token: &SyntaxToken| {
            errors
                .iter()
                .all(|e| e.range.intersect(token.text_range()).is_none())
        };
        let mut comments = Vec::new();
        let mut doc_comments = 0;
        let mut body = Vec::new();
        for token in root
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
        {
            match token.kind() {
                SyntaxKind::LineComment | SyntaxKind::BlockComment => {
                    comments.push(letters(&token))
                }
                SyntaxKind::DocComment => doc_comments += 1,
                SyntaxKind::LParen | SyntaxKind::RParen => {}
                kind if kind.is_trivia() || in_header(&token) => {}
                // A negative number is one literal, as wrapping can print a
                // positive one.
                SyntaxKind::Minus if token.next_token().is_some_and(|t| sign_of(&t).is_some()) => {}
                _ if is_literal(&token) && !valid(&token) => {
                    body.push(Token::Other(token.kind(), token.text().to_owned()));
                }
                _ if is_number(&token) => {
                    let value = literal_token(&token);
                    body.push(Token::Literal(match sign_of(&token) {
                        Some(_) => negative(value),
                        None => value,
                    }));
                }
                // Elm 0.18's `[a..b]` becomes `List.range a b`, a record
                // type's `=` a `:`, a record's `:` an `=`, and a case's
                // `then` an `of`.
                SyntaxKind::LBracket if parent_is(&token, SyntaxKind::RangeExpr) => {
                    body.extend([
                        Token::Other(SyntaxKind::UpperIdent, "List".to_owned()),
                        Token::Other(SyntaxKind::Dot, ".".to_owned()),
                        Token::Other(SyntaxKind::LowerIdent, "range".to_owned()),
                    ]);
                }
                SyntaxKind::DotDot | SyntaxKind::RBracket
                    if parent_is(&token, SyntaxKind::RangeExpr) => {}
                SyntaxKind::Eq if parent_is(&token, SyntaxKind::FieldType) => {
                    body.push(Token::Other(SyntaxKind::Colon, ":".to_owned()));
                }
                SyntaxKind::Colon if parent_is(&token, SyntaxKind::Field) => {
                    body.push(Token::Other(SyntaxKind::Eq, "=".to_owned()));
                }
                SyntaxKind::ThenKw if parent_is(&token, SyntaxKind::CaseExpr) => {
                    body.push(Token::Other(SyntaxKind::OfKw, "of".to_owned()));
                }
                SyntaxKind::Char | SyntaxKind::String | SyntaxKind::MultilineString => {
                    body.push(Token::Literal(literal_token(&token)));
                }
                // elm-format drops a shader's carriage returns.
                SyntaxKind::Glsl => {
                    body.push(Token::Other(
                        SyntaxKind::Glsl,
                        token.text().replace('\r', ""),
                    ));
                }
                kind => body.push(Token::Other(kind, token.text().to_owned())),
            }
        }
        comments.sort();
        let imports = root
            .children()
            .filter_map(ast::Import::cast)
            .filter_map(|import| import.module_name())
            .map(|name| {
                name.syntax()
                    .descendants_with_tokens()
                    .filter_map(|e| e.into_token())
                    .filter(|t| !t.kind().is_trivia())
                    .map(|t| t.text().to_owned())
                    .collect()
            })
            .collect();
        Summary {
            comments,
            doc_comments,
            imports,
            body,
        }
    }
}

/// Whether `token` is in the module header or an import.
fn in_header(token: &SyntaxToken) -> bool {
    token
        .parent_ancestors()
        .any(|n| matches!(n.kind(), SyntaxKind::ModuleHeader | SyntaxKind::Import))
}

/// A comment's text without its whitespace, which formatting changes.
fn letters(comment: &SyntaxToken) -> String {
    comment.text().split_whitespace().collect()
}

fn is_literal(token: &SyntaxToken) -> bool {
    is_number(token)
        || matches!(
            token.kind(),
            SyntaxKind::Char | SyntaxKind::String | SyntaxKind::MultilineString
        )
}

fn is_number(token: &SyntaxToken) -> bool {
    matches!(token.kind(), SyntaxKind::Int | SyntaxKind::Float)
}

/// The `-` touching `number` that negates it, in a pattern's literal or
/// a negation, which lowering reads as part of it; a `-` in an operator
/// chain subtracts.
fn sign_of(number: &SyntaxToken) -> Option<SyntaxToken> {
    let minus = number.prev_token()?;
    let negates = minus.parent() == number.parent()
        || minus
            .parent()
            .is_some_and(|p| p.kind() == SyntaxKind::NegateExpr);
    (is_number(number) && minus.kind() == SyntaxKind::Minus && negates).then_some(minus)
}

fn parent_is(token: &SyntaxToken, kind: SyntaxKind) -> bool {
    token.parent().is_some_and(|p| p.kind() == kind)
}
