//! What formatting must keep for every module it accepts: the fixtures,
//! the property tests, the fuzz target and the corpus check these.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{
    SyntaxError, SyntaxKind, SyntaxNode, SyntaxToken, Tool, child_token, parse_module,
    significant_tokens, tokens, validate,
};

use crate::format;
use crate::lower::{is_number, is_text, literal_token, negative};
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
    /// Every invariant, in the order reports list them.
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

/// A name that is no invariant's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownInvariant(pub String);

impl fmt::Display for UnknownInvariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "no invariant `{}`", self.0)
    }
}

impl std::error::Error for UnknownInvariant {}

impl FromStr for Invariant {
    type Err = UnknownInvariant;

    /// The invariant [`Invariant::name`] names.
    fn from_str(name: &str) -> Result<Invariant, UnknownInvariant> {
        Invariant::ALL
            .into_iter()
            .find(|i| i.name() == name)
            .ok_or_else(|| UnknownInvariant(name.to_owned()))
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

/// Why formatting `text` breaks invariants other than `listed`, those
/// elm-format breaks there; `None` where it breaks just those, or `text`
/// isn't UTF-8.
pub fn invariant_mismatch(text: &[u8], listed: &[Invariant]) -> Option<String> {
    let found = std::str::from_utf8(text).map_or_else(|_| Vec::new(), broken_invariants);
    let broken: Vec<Invariant> = found.iter().map(|(i, _)| *i).collect();
    let names = |invariants: &[Invariant]| {
        let names: Vec<_> = invariants.iter().map(|i| i.name()).collect();
        names.join(",")
    };
    match found.first() {
        _ if broken == listed => None,
        Some((invariant, how)) if listed.is_empty() => {
            Some(format!("breaks `{}`: {how}", invariant.name()))
        }
        _ => Some(format!(
            "breaks `{}`, where invariants.tsv lists `{}`",
            names(&broken),
            names(listed),
        )),
    }
}

/// The invariants elm-format 0.8.8 breaks for some modules like `text`,
/// in the order of [`Invariant::ALL`], as nomi does, by what `text`
/// holds:
///
/// - comments: comments in the module header and exposing lists vanish,
///   and comments and doc comments' Markdown can move or respace on a
///   second run;
/// - `\x` escapes, which elm-format reads as Elm 0.18 did and prints as
///   other code points, some that don't exist;
/// - a call spanning lines of a parenthesized call or of a range, as in
///   `(f x) y` or `[a..b] y` then an argument below, whose function's
///   arguments join the first line and move below it on a second run;
/// - an Elm 0.18 infix declaration spanning lines for a comment in it,
///   which elm-format prints as a request to report it, which doesn't
///   parse;
/// - a negated zero in parentheses, `-(0)`, which elm-format prints as
///   `-0`, and a second run as `0`;
/// - a module without a header, declaring a type alias and a custom
///   type of one name, which the exposing list elm-format writes names
///   twice and a second run once.
///
/// The fixtures' and the corpus's `invariants.tsv` record the modules.
pub fn upstream_breaks(text: &str) -> Vec<Invariant> {
    let root = parse_module(text).syntax();
    let broken: Vec<Invariant> = UPSTREAM_BREAKS
        .iter()
        .filter(|kind| (kind.holds)(&root))
        .flat_map(|kind| kind.breaks.iter().copied())
        .collect();
    Invariant::ALL
        .into_iter()
        .filter(|i| broken.contains(i))
        .collect()
}

/// What of a module makes elm-format break invariants, and which.
struct UpstreamBreak {
    holds: fn(&SyntaxNode) -> bool,
    breaks: &'static [Invariant],
}

/// Each kind of module [`upstream_breaks`] lists.
const UPSTREAM_BREAKS: &[UpstreamBreak] = &[
    UpstreamBreak {
        holds: has_comments,
        breaks: &[Invariant::KeepsComments, Invariant::Idempotent],
    },
    UpstreamBreak {
        holds: has_hex_escape,
        breaks: &[
            Invariant::Parses,
            Invariant::KeepsTokens,
            Invariant::Idempotent,
        ],
    },
    UpstreamBreak {
        holds: multiline_call_of_call_or_range,
        breaks: &[Invariant::Idempotent],
    },
    UpstreamBreak {
        holds: legacy_infix_with_comment,
        breaks: &[Invariant::Parses, Invariant::KeepsTokens],
    },
    UpstreamBreak {
        holds: negated_parenthesized_zero,
        breaks: &[Invariant::KeepsTokens, Invariant::Idempotent],
    },
    UpstreamBreak {
        holds: headless_alias_and_type_share_name,
        breaks: &[Invariant::Idempotent],
    },
];

fn has_comments(root: &SyntaxNode) -> bool {
    tokens(root).any(|t| {
        matches!(
            t.kind(),
            SyntaxKind::LineComment | SyntaxKind::BlockComment | SyntaxKind::DocComment
        )
    })
}

/// Whether a character or string holds a `\x` escape.
fn has_hex_escape(root: &SyntaxNode) -> bool {
    tokens(root).any(|t| is_text(t.kind()) && t.text().contains("\\x"))
}

/// Whether a call spanning lines calls a range, or a call or a range in
/// parentheses.
fn multiline_call_of_call_or_range(root: &SyntaxNode) -> bool {
    root.descendants().any(|call| {
        call.kind() == SyntaxKind::CallExpr
            && call.text().to_string().trim().contains('\n')
            && call.first_child().is_some_and(|function| {
                function.kind() == SyntaxKind::RangeExpr
                    || function.kind() == SyntaxKind::ParenExpr
                        && function.children().any(|inner| {
                            matches!(inner.kind(), SyntaxKind::CallExpr | SyntaxKind::RangeExpr)
                        })
            })
    })
}

/// Whether an Elm 0.18 infix declaration spans lines for a comment in it.
fn legacy_infix_with_comment(root: &SyntaxNode) -> bool {
    root.descendants()
        .filter(|decl| decl.kind() == SyntaxKind::LegacyInfixDecl)
        .flat_map(|decl| tokens(&decl))
        .any(|t| {
            t.kind() == SyntaxKind::LineComment
                || t.kind() == SyntaxKind::BlockComment && t.text().contains('\n')
        })
}

/// Whether a zero is negated in parentheses, as in `-(0)` or `-(-0)`.
fn negated_parenthesized_zero(root: &SyntaxNode) -> bool {
    tokens(root).any(|minus| {
        parent_is(&minus, SyntaxKind::NegateExpr)
            && minus
                .next_token()
                .is_some_and(|t| t.kind() == SyntaxKind::LParen)
            && std::iter::successors(minus.next_token(), |t| t.next_token())
                .find(|t| {
                    !matches!(t.kind(), SyntaxKind::LParen | SyntaxKind::Minus)
                        && !t.kind().is_trivia()
                })
                .is_some_and(|zero| {
                    zero.kind() == SyntaxKind::Int
                        && matches!(literal_token(&zero), LiteralValue::Int(0, _))
                })
    })
}

/// Whether a module without a header declares a type alias and a custom
/// type of one name.
fn headless_alias_and_type_share_name(root: &SyntaxNode) -> bool {
    let type_names = |kind| {
        root.children()
            .filter(move |decl| decl.kind() == kind)
            .filter_map(|decl| child_token(&decl, SyntaxKind::UpperIdent))
            .map(|name| name.text().to_owned())
            .collect::<BTreeSet<_>>()
    };
    !root
        .children()
        .any(|n| n.kind() == SyntaxKind::ModuleHeader)
        && !type_names(SyntaxKind::TypeAliasDecl)
            .is_disjoint(&type_names(SyntaxKind::CustomTypeDecl))
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
        let mut comments = Vec::new();
        let mut doc_comments = 0;
        let mut body = Body::default();
        for token in tokens(root) {
            match token.kind() {
                SyntaxKind::LineComment | SyntaxKind::BlockComment => {
                    comments.push(letters(&token));
                }
                SyntaxKind::DocComment => doc_comments += 1,
                SyntaxKind::LParen | SyntaxKind::RParen => {}
                kind if kind.is_trivia() || in_header(&token) => {}
                _ => body.push(&token, errors),
            }
        }
        comments.sort();
        let imports = root
            .children()
            .filter_map(ast::Import::cast)
            .filter_map(|import| import.module_name())
            .map(|name| {
                significant_tokens(name.syntax())
                    .map(|t| t.text().to_owned())
                    .collect()
            })
            .collect();
        Summary {
            comments,
            doc_comments,
            imports,
            body: body.tokens,
        }
    }
}

/// The declarations' tokens, as formatting must keep them.
#[derive(Default)]
struct Body {
    tokens: Vec<Token>,
    /// Where in `tokens` the last `-` that negates went.
    negation: Option<usize>,
}

impl Body {
    /// Adds what `token`, a token of the declarations that is neither
    /// trivia nor a parenthesis, stands for, where `errors` are its
    /// parse's.
    fn push(&mut self, token: &SyntaxToken, errors: &[SyntaxError]) {
        let tokens = &mut self.tokens;
        let other = |kind, text: &str| Token::Other(kind, text.to_owned());
        match token.kind() {
            // A negative number is one literal, as wrapping can print a
            // positive one and upstream drops the parentheses of `-(1)`.
            SyntaxKind::Minus if negates(token) => {
                self.negation = Some(tokens.len());
                tokens.push(other(SyntaxKind::Minus, "-"));
            }
            kind if (is_number(kind) || is_text(kind))
                && errors
                    .iter()
                    .any(|e| e.range.intersect(token.text_range()).is_some()) =>
            {
                tokens.push(other(kind, token.text()));
            }
            kind if is_number(kind) => {
                let value = literal_token(token);
                let value = if self.negation.take().is_some_and(|i| i + 1 == tokens.len()) {
                    tokens.pop();
                    negative(value)
                } else {
                    value
                };
                tokens.push(Token::Literal(value));
            }
            // Elm 0.18's `[a..b]` becomes `List.range a b`, a record
            // type's `=` a `:`, a record's `:` an `=`, and a case's
            // `then` an `of`.
            SyntaxKind::LBracket if parent_is(token, SyntaxKind::RangeExpr) => {
                tokens.extend([
                    other(SyntaxKind::UpperIdent, "List"),
                    other(SyntaxKind::Dot, "."),
                    other(SyntaxKind::LowerIdent, "range"),
                ]);
            }
            SyntaxKind::DotDot | SyntaxKind::RBracket
                if parent_is(token, SyntaxKind::RangeExpr) => {}
            SyntaxKind::Eq if parent_is(token, SyntaxKind::FieldType) => {
                tokens.push(other(SyntaxKind::Colon, ":"));
            }
            SyntaxKind::Colon if parent_is(token, SyntaxKind::Field) => {
                tokens.push(other(SyntaxKind::Eq, "="));
            }
            // elm-format reads `→` and `=>` as `->`.
            SyntaxKind::Arrow => tokens.push(other(SyntaxKind::Arrow, "->")),
            SyntaxKind::ThenKw if parent_is(token, SyntaxKind::CaseExpr) => {
                tokens.push(other(SyntaxKind::OfKw, "of"));
            }
            kind if is_text(kind) => tokens.push(Token::Literal(literal_token(token))),
            // elm-format drops a shader's carriage returns.
            SyntaxKind::Glsl => tokens.push(Token::Other(
                SyntaxKind::Glsl,
                token.text().replace('\r', ""),
            )),
            kind => tokens.push(other(kind, token.text())),
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

/// Whether `minus` negates what follows: a negation's, or a pattern's
/// literal's; a `-` in an operator chain subtracts.
fn negates(minus: &SyntaxToken) -> bool {
    minus
        .parent()
        .is_some_and(|p| p.kind() == SyntaxKind::NegateExpr)
        || minus
            .next_token()
            .is_some_and(|number| is_number(number.kind()) && number.parent() == minus.parent())
}

fn parent_is(token: &SyntaxToken, kind: SyntaxKind) -> bool {
    token.parent().is_some_and(|p| p.kind() == kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invariants_parse_from_their_names() {
        for invariant in Invariant::ALL {
            assert_eq!(invariant.name().parse(), Ok(invariant));
        }
        assert_eq!(
            "nonsense".parse::<Invariant>(),
            Err(UnknownInvariant("nonsense".to_owned()))
        );
    }

    #[test]
    fn invariant_mismatches_follow_the_table() {
        let formatted = b"module Main exposing (x)\n\n\nx =\n    1\n";
        assert_eq!(invariant_mismatch(formatted, &[]), None);
        assert!(invariant_mismatch(formatted, &[Invariant::Idempotent]).is_some());
        assert_eq!(invariant_mismatch(b"x =", &[]), None);
        assert_eq!(invariant_mismatch(b"\xff", &[]), None);
    }
}
