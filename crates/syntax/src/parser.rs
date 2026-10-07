//! The parser's machinery: a cursor over significant tokens, markers that
//! record a flat event list, and the depth guard. Grammar functions build
//! on this; [`build`] turns the events into a rowan tree.
//!
//! The parser never sees trivia. Whitespace and comments wait in the token
//! stream, and the builder places them, so no grammar rule can forget one.
//!
//! Tokens are lexed on demand. Elm reads some symbols a character at a
//! time, so `x =-1` is `x = -1`, while the lexer sees one operator `=-`;
//! the grammar splits such a token where it expects the symbol, and only
//! the few tokens already peeked past it are lexed again. Splitting stays
//! linear however many tokens it splits.

mod build;

use std::collections::VecDeque;
use std::num::NonZeroU32;

use nomi_base::{TextRange, TextSize};
use rowan::GreenNode;

use crate::lexer::{LexErrorKind, Lexer, Token};
use crate::{SyntaxKind, SyntaxNode};

/// A lossless tree and every problem found building it, in source order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parse {
    green: GreenNode,
    errors: Vec<SyntaxError>,
    /// Tokens elm wants indented that elm-format takes anywhere. Only the
    /// parser knows the indentation context, so it records them for
    /// [`crate::validate`].
    pub(crate) unindented: Vec<TextRange>,
}

impl Parse {
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }

    pub fn green(&self) -> &GreenNode {
        &self.green
    }

    pub fn errors(&self) -> &[SyntaxError] {
        &self.errors
    }

    /// The tree as indented `Kind@start..end` lines, tokens with their
    /// text; for tests and fixtures.
    pub fn debug_tree(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        let mut depth = 0;
        for event in self.syntax().preorder_with_tokens() {
            match event {
                rowan::WalkEvent::Enter(element) => {
                    let range = element.text_range();
                    let indent = "  ".repeat(depth);
                    let _ = match &element {
                        rowan::NodeOrToken::Node(node) => {
                            depth += 1;
                            writeln!(out, "{indent}{:?}@{range:?}", node.kind())
                        }
                        rowan::NodeOrToken::Token(token) => writeln!(
                            out,
                            "{indent}{:?}@{range:?} {:?}",
                            token.kind(),
                            token.text()
                        ),
                    };
                }
                rowan::WalkEvent::Leave(rowan::NodeOrToken::Node(_)) => depth -= 1,
                rowan::WalkEvent::Leave(rowan::NodeOrToken::Token(_)) => {}
            }
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    pub kind: SyntaxErrorKind,
    pub range: TextRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxErrorKind {
    Lexical(LexErrorKind),
    /// The grammar needed this token here.
    Expected(SyntaxKind),
    /// Something that can appear in an exposing list.
    ExpectedExposed,
    /// A token that continues a construct sits left of the column the
    /// construct allows.
    ExpectedIndent,
    /// A type.
    ExpectedType,
    /// A pattern.
    ExpectedPattern,
    /// An expression.
    ExpectedExpression,
    /// A declaration: a value, type, type alias, port or infix.
    ExpectedDeclaration,
    /// A name joined to a wildcard, as in `_unused`.
    NamedWildcard,
    /// A token that must start its line follows other code on that line.
    ExpectedFreshLine,
    /// A token that cannot go here, skipped.
    Unexpected,
    /// Nesting went past [`DEPTH_LIMIT`].
    TooDeep,

    // Found by [`crate::validate`]: the grammar takes these, a tool doesn't.
    /// A top-level `( a, b ) = pair`. elm wants a name.
    DestructureDeclaration,
    /// A definition named differently from the signature above it.
    DefinitionNameMismatch,
    /// The legacy `(,)` tuple constructor.
    TupleConstructor,
    /// The legacy `=` in a record type, as in `{ name = String }`.
    LegacyFieldType,
    /// The legacy `:` in a record, as in `{ name : "Tom" }`.
    LegacyField,
    /// A negative number in a pattern.
    NegativePattern,
    /// A float in a pattern.
    FloatPattern,
    /// A module header without an exposing list.
    MissingExposing,
    /// An exposing list without `exposing` or without parentheses.
    LegacyExposing,
    /// The legacy `Type(A, B)` in an exposing list. elm wants `Type(..)`.
    LegacyExposedVariants,
    /// The legacy `where` after a module's name or exposing list.
    LegacyWhere,
    /// An effect module without `where { command = … }`.
    ExpectedEffectFields,
    /// `where { … }` after a module that isn't an effect module.
    UnexpectedEffectFields,
    /// An effect field other than one `command` and one `subscription`.
    BadEffectField,
    /// An effect module outside a kernel package.
    EffectModuleOutsideKernel,
    /// A port module in a package.
    PortModuleInPackage,
    /// A port in a package.
    PortInPackage,
    /// A port in a module that isn't a port module.
    UnexpectedPort,
    /// A port module without ports.
    NoPorts,
    /// An infix declaration outside a kernel package, or after another
    /// declaration.
    UnexpectedInfix,
    /// A module without a value, type, type alias or port declaration.
    NoDeclarations,
    /// The file ends on the line of an import.
    ImportWithoutNewline,
    /// Whitespace inside an operator's parentheses, as in `( + )`.
    SpacedOperator,
    /// An infix precedence other than one decimal digit, such as `10`.
    BadPrecedence,
    /// An expression of a code block, after the first, starting with
    /// something other than a letter or `_`.
    ExpectedLetterStart,
}

/// How deeply grammar rules may nest. Far beyond real code, which rarely
/// passes 50, but generated code can nest thousands deep.
/// It bounds the tree's depth, which matters past the parser: rowan drops
/// trees recursively, and [`nomi_base::STACK_SIZE`] is sized for the
/// deepest tree this admits.
pub(crate) const DEPTH_LIMIT: u32 = 25_000;

/// Stack kept free before a guarded rule runs, and the size of each new
/// segment when it isn't.
const RED_ZONE: usize = 128 * 1024;
const STACK_SEGMENT: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    /// Opens a node. `forward_parent` points at a later `Start` that
    /// [`CompletedMarker::precede`] made this node's parent.
    Start {
        kind: SyntaxKind,
        forward_parent: Option<NonZeroU32>,
    },
    /// The next significant token, as `kind`: usually its lexed kind, or a
    /// contextual keyword.
    Token {
        kind: SyntaxKind,
    },
    Finish,
    /// A `Start` that was abandoned or already moved.
    Tombstone,
}

/// A token lexed but not yet consumed.
#[derive(Clone, Copy, Debug)]
struct Ahead {
    token: Token,
    /// The column it starts at.
    column: u32,
    /// How many lexical errors were found up to and including it.
    lex_errors: u32,
}

pub(crate) struct Parser<'t> {
    text: &'t str,
    lexer: Lexer<'t>,
    /// The column at the lexer's offset.
    lexer_column: u32,
    /// Tokens lexed but not consumed, next first: the lookahead.
    ahead: VecDeque<Ahead>,
    /// Tokens lexed, those lexed again after a split included.
    lexed: usize,
    /// Tokens consumed so far, trivia included, in source order.
    consumed: Vec<Token>,
    events: Vec<Event>,
    errors: Vec<SyntaxError>,
    depth: u32,
    /// Set once nesting passed [`DEPTH_LIMIT`]. The rest of the input is
    /// then one error node, and rules unwinding from there report nothing.
    gave_up: bool,
    /// The leftmost column a continuation token may start at: one past
    /// the enclosing construct's column, or 0 in a fragment.
    min_column: u32,
    /// Where a token starts that its block placed by alignment, such as a
    /// case branch's first token. It counts as indented.
    anchor: Option<TextSize>,
    unindented: Vec<TextRange>,
}

impl<'t> Parser<'t> {
    pub(crate) fn new(text: &'t str) -> Parser<'t> {
        Parser {
            text,
            lexer: Lexer::new(text),
            lexer_column: 0,
            ahead: VecDeque::new(),
            lexed: 0,
            consumed: Vec::new(),
            events: Vec::new(),
            errors: Vec::new(),
            depth: 0,
            gave_up: false,
            min_column: 1,
            anchor: None,
            unindented: Vec::new(),
        }
    }

    /// Builds the tree. Every token must have been consumed, except
    /// trailing trivia.
    pub(crate) fn finish(mut self) -> Parse {
        while let Some(Ahead { token, .. }) = self.ahead_at(0) {
            assert!(
                token.kind.is_trivia(),
                "the grammar left {:?} unconsumed",
                token.kind
            );
            self.ahead.pop_front();
            self.consumed.push(token);
        }
        let mut errors: Vec<SyntaxError> = (self.lexer.into_errors().into_iter())
            .map(|e| SyntaxError {
                kind: SyntaxErrorKind::Lexical(e.kind),
                range: e.range,
            })
            .collect();
        errors.extend(self.errors);
        // A stable sort keeps lexical errors before syntax errors at the
        // same place.
        errors.sort_by_key(|e| e.range.start());
        let green = build::build(self.text, &self.consumed, self.events);
        Parse {
            green,
            errors,
            unindented: self.unindented,
        }
    }

    /// How many tokens were lexed, counting each time one is lexed again
    /// after a split.
    #[cfg(test)]
    pub(crate) fn lexed(&self) -> usize {
        self.lexed
    }

    /// The `i`th token ahead, trivia included, lexing up to it.
    fn ahead_at(&mut self, i: usize) -> Option<Ahead> {
        while self.ahead.len() <= i {
            let token = self.lexer.next_token()?;
            let column = self.lexer_column;
            self.lexer_column = advance_column(column, &self.text[token.range]);
            self.lexed += 1;
            self.ahead.push_back(Ahead {
                token,
                column,
                lex_errors: self.lexer.errors().len() as u32,
            });
        }
        Some(self.ahead[i])
    }

    /// The `n`th significant token ahead, and its index in the lookahead.
    fn nth_ahead(&mut self, n: usize) -> Option<(usize, Ahead)> {
        let mut seen = 0;
        for i in 0.. {
            let a = self.ahead_at(i)?;
            if a.token.kind.is_trivia() {
                continue;
            }
            if seen == n {
                return Some((i, a));
            }
            seen += 1;
        }
        unreachable!("the lookahead ends")
    }

    fn nth_token(&mut self, n: usize) -> Option<Ahead> {
        self.nth_ahead(n).map(|(_, a)| a)
    }

    /// The `n`th significant token ahead, or [`SyntaxKind::Eof`].
    pub(crate) fn nth(&mut self, n: usize) -> SyntaxKind {
        self.nth_token(n).map_or(SyntaxKind::Eof, |a| a.token.kind)
    }

    /// Whether the `n`th token ahead is the lower identifier spelling the
    /// contextual keyword `kind`.
    pub(crate) fn nth_at_contextual(&mut self, n: usize, kind: SyntaxKind) -> bool {
        debug_assert!(kind.is_contextual_keyword());
        self.nth_token(n).is_some_and(|a| {
            a.token.kind == SyntaxKind::LowerIdent && Some(&self.text[a.token.range]) == kind.text()
        })
    }

    /// Whether the `n`th token ahead is `kind`, or an operator that starts
    /// with it, as `=-` starts with `=`. Where elm's grammar expects a
    /// symbol such as `=`, `->` or `|`, it reads just its characters;
    /// [`Parser::bump_symbol`] splits the operator.
    pub(crate) fn nth_at_symbol(&mut self, n: usize, kind: SyntaxKind) -> bool {
        self.nth_token(n).is_some_and(|a| {
            a.token.kind == kind
                || (is_operator_token(a.token.kind)
                    && kind
                        .text()
                        .is_some_and(|symbol| self.text[a.token.range].starts_with(symbol)))
        })
    }

    pub(crate) fn at_symbol(&mut self, kind: SyntaxKind) -> bool {
        self.nth_at_symbol(0, kind)
    }

    /// Consumes the symbol `kind`, which [`Parser::at_symbol`] found,
    /// splitting it off the operator it starts. The rest is lexed again:
    /// `=--` leaves a comment, not an operator `--`.
    pub(crate) fn bump_symbol(&mut self, kind: SyntaxKind) {
        debug_assert!(self.at_symbol(kind));
        if !self.at(kind) {
            let (i, a) = self.nth_ahead(0).expect("at a symbol");
            let len = TextSize::of(kind.text().expect("a symbol has a fixed text"));
            let cut = a.token.range.start() + len;
            self.ahead.truncate(i + 1);
            self.ahead[i].token = Token {
                kind,
                range: TextRange::new(a.token.range.start(), cut),
            };
            self.lexer.rewind(cut, a.lex_errors as usize);
            // Operators are ASCII, so bytes are columns.
            self.lexer_column = a.column + u32::from(len);
        }
        self.bump();
    }

    /// Whether the `n`th token ahead directly follows the token before
    /// it, with no trivia between them.
    pub(crate) fn nth_is_joined(&mut self, n: usize) -> bool {
        let Some((i, _)) = self.nth_ahead(n) else {
            return false;
        };
        i == 0 || !self.ahead[i - 1].token.kind.is_trivia()
    }

    pub(crate) fn current(&mut self) -> SyntaxKind {
        self.nth(0)
    }

    pub(crate) fn at(&mut self, kind: SyntaxKind) -> bool {
        self.current() == kind
    }

    /// The current token's text; empty at the end.
    pub(crate) fn current_text(&mut self) -> &'t str {
        let range = self.current_range();
        &self.text[range]
    }

    /// The current token's range; empty at the end of the text.
    pub(crate) fn current_range(&mut self) -> TextRange {
        self.nth_token(0).map_or_else(
            || TextRange::empty(TextSize::of(self.text)),
            |a| a.token.range,
        )
    }

    /// The column the current token starts at, counting characters; 0 at
    /// the end.
    pub(crate) fn current_column(&mut self) -> u32 {
        self.nth_token(0).map_or(0, |a| a.column)
    }

    /// Whether the current token starts its line. The end counts as one.
    pub(crate) fn at_fresh_line(&mut self) -> bool {
        self.current_column() == 0
    }

    /// Whether the current token is indented enough to continue the
    /// construct being parsed, or is the anchor. The end never is.
    pub(crate) fn at_indented(&mut self) -> bool {
        let Some(a) = self.nth_token(0) else {
            return false;
        };
        a.column >= self.min_column || self.anchor == Some(a.token.range.start())
    }

    /// Lets the current token start an item of a block it is aligned in,
    /// at the column where everything after it must be indented further.
    pub(crate) fn anchor(&mut self) {
        self.anchor = self.nth_token(0).map(|a| a.token.range.start());
    }

    pub(crate) fn unanchor(&mut self) {
        self.anchor = None;
    }

    /// Runs `rule` with continuations allowed from `column` on.
    pub(crate) fn with_min_column<R>(
        &mut self,
        column: u32,
        rule: impl FnOnce(&mut Parser<'t>) -> R,
    ) -> R {
        let outer = std::mem::replace(&mut self.min_column, column);
        let result = rule(self);
        self.min_column = outer;
        result
    }

    /// Reports a continuation token that isn't indented. Parsing goes on as
    /// if it were.
    pub(crate) fn check_indented(&mut self) {
        if !self.at(SyntaxKind::Eof) && !self.at_indented() {
            self.error(SyntaxErrorKind::ExpectedIndent);
        }
    }

    /// Records a continuation token that isn't indented, where only elm
    /// minds. Parsing goes on as if it were.
    pub(crate) fn note_unindented(&mut self) {
        if !self.at(SyntaxKind::Eof) && !self.at_indented() {
            let range = self.current_range();
            self.unindented.push(range);
        }
    }

    /// Consumes the current token as its own kind.
    pub(crate) fn bump(&mut self) {
        let kind = self.current();
        self.bump_as(kind);
    }

    /// Consumes the current token as `kind`, such as `alias` as
    /// [`SyntaxKind::AliasKw`].
    pub(crate) fn bump_as(&mut self, kind: SyntaxKind) {
        assert!(!self.at(SyntaxKind::Eof), "bumped past the end");
        while let Some(Ahead { token, .. }) = self.ahead.pop_front() {
            self.consumed.push(token);
            if !token.kind.is_trivia() {
                break;
            }
        }
        self.events.push(Event::Token { kind });
    }

    pub(crate) fn eat(&mut self, kind: SyntaxKind) -> bool {
        let at = self.at(kind);
        if at {
            self.bump();
        }
        at
    }

    /// Consumes `kind`, or reports that it is missing.
    pub(crate) fn expect(&mut self, kind: SyntaxKind) -> bool {
        let eaten = self.eat(kind);
        if !eaten {
            self.error(SyntaxErrorKind::Expected(kind));
        }
        eaten
    }

    /// Reports a problem at the current token, or at the end. Only the
    /// first problem at a token counts: a construct that ends early leaves
    /// the expectations of those around it unmet at the same place.
    pub(crate) fn error(&mut self, kind: SyntaxErrorKind) {
        if self.gave_up {
            return;
        }
        let range = self.current_range();
        if self.errors.last().is_some_and(|e| e.range == range) {
            return;
        }
        self.errors.push(SyntaxError { kind, range });
    }

    /// Reports a problem and skips the current token inside an
    /// [`SyntaxKind::Error`] node. At the end it only reports.
    pub(crate) fn error_and_bump(&mut self, kind: SyntaxErrorKind) {
        self.error(kind);
        if !self.at(SyntaxKind::Eof) {
            let m = self.start();
            self.bump();
            m.complete(self, SyntaxKind::Error);
        }
    }

    /// An empty [`SyntaxKind::Missing`] node where something was expected.
    pub(crate) fn missing(&mut self) -> CompletedMarker {
        self.start().complete(self, SyntaxKind::Missing)
    }

    pub(crate) fn start(&mut self) -> Marker {
        let pos = self.events.len() as u32;
        self.events.push(Event::Tombstone);
        Marker { pos }
    }

    /// Runs a grammar rule that may recurse. Past [`DEPTH_LIMIT`] it
    /// reports [`SyntaxErrorKind::TooDeep`], gives up on the rest of the
    /// input, and returns `None` without running `rule`. Below it, the
    /// stack grows as needed, so depth is bounded by memory, not by the
    /// thread's stack.
    pub(crate) fn guarded<R>(&mut self, rule: impl FnOnce(&mut Parser<'t>) -> R) -> Option<R> {
        if self.depth >= DEPTH_LIMIT {
            self.error(SyntaxErrorKind::TooDeep);
            self.gave_up = true;
            let m = self.start();
            while !self.at(SyntaxKind::Eof) {
                self.bump();
            }
            m.complete(self, SyntaxKind::Error);
            return None;
        }
        self.depth += 1;
        let result = stacker::maybe_grow(RED_ZONE, STACK_SEGMENT, || rule(self));
        self.depth -= 1;
        Some(result)
    }
}

/// The column after `text`, starting at `column`. Columns count
/// characters, ignoring carriage returns as elm's `eatSpaces` does.
// TODO(plan 1.17): confirm the unit against the oracle's regions.
fn advance_column(column: u32, text: &str) -> u32 {
    let (base, rest) = match text.rsplit_once('\n') {
        Some((_, rest)) => (0, rest),
        None => (column, text),
    };
    let width = rest.chars().filter(|&c| c != '\r').count();
    base + width as u32
}

/// An open node. Complete or abandon it; a forgotten marker leaves no
/// node.
#[must_use]
#[derive(Debug)]
pub(crate) struct Marker {
    pos: u32,
}

impl Marker {
    pub(crate) fn complete(self, p: &mut Parser<'_>, kind: SyntaxKind) -> CompletedMarker {
        assert!(kind.is_node(), "{kind:?} is not a node kind");
        let event = &mut p.events[self.pos as usize];
        debug_assert_eq!(*event, Event::Tombstone);
        *event = Event::Start {
            kind,
            forward_parent: None,
        };
        p.events.push(Event::Finish);
        CompletedMarker { pos: self.pos }
    }

    /// Drops the node; its tokens and children go to its parent.
    pub(crate) fn abandon(self, p: &mut Parser<'_>) {
        if self.pos as usize == p.events.len() - 1 {
            p.events.pop();
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CompletedMarker {
    pos: u32,
}

impl CompletedMarker {
    /// Opens a node that will wrap this one, as `a` becomes the left side
    /// of `a + b` once the parser sees `+`.
    pub(crate) fn precede(self, p: &mut Parser<'_>) -> Marker {
        let parent = p.start();
        let Event::Start { forward_parent, .. } = &mut p.events[self.pos as usize] else {
            unreachable!("a completed marker points at its Start");
        };
        *forward_parent = NonZeroU32::new(parent.pos - self.pos);
        parent
    }
}

#[cfg(test)]
mod tests;

/// Whether the lexer's operator rule made a token of `kind`, so that a
/// symbol may start it.
fn is_operator_token(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::Operator
            | SyntaxKind::Dot
            | SyntaxKind::DotDot
            | SyntaxKind::Arrow
            | SyntaxKind::Eq
            | SyntaxKind::Colon
            | SyntaxKind::ColonColon
            | SyntaxKind::Pipe
            | SyntaxKind::Minus
    )
}
