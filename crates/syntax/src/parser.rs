//! The parser's machinery: a cursor over significant tokens, markers that
//! record a flat event list, and the depth guard. Grammar functions build
//! on this; [`build`] turns the events into a rowan tree.
//!
//! The parser never sees trivia. Whitespace and comments wait in the token
//! stream, and the builder places them, so no grammar rule can forget one.

mod build;

use std::num::NonZeroU32;

use nomi_base::{TextRange, TextSize};
use rowan::GreenNode;

use crate::lexer::{LexErrorKind, Token};
use crate::{SyntaxKind, SyntaxNode, lex};

/// A lossless tree and every problem found building it, in source order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parse {
    green: GreenNode,
    errors: Vec<SyntaxError>,
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
    /// Nesting went past [`DEPTH_LIMIT`].
    TooDeep,
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

pub(crate) struct Parser<'t> {
    text: &'t str,
    /// Tokens not yet consumed, reversed, so the next one is last. A split
    /// replaces only the top, so it costs nothing however far in it is.
    ahead: Vec<Token>,
    /// Tokens consumed so far, trivia included, in source order.
    consumed: Vec<Token>,
    events: Vec<Event>,
    errors: Vec<SyntaxError>,
    depth: u32,
}

impl<'t> Parser<'t> {
    pub(crate) fn new(text: &'t str) -> Parser<'t> {
        let (mut tokens, lex_errors) = lex(text).into_parts();
        tokens.reverse();
        Parser {
            text,
            ahead: tokens,
            consumed: Vec::new(),
            events: Vec::new(),
            errors: lex_errors
                .into_iter()
                .map(|e| SyntaxError {
                    kind: SyntaxErrorKind::Lexical(e.kind),
                    range: e.range,
                })
                .collect(),
            depth: 0,
        }
    }

    /// Builds the tree. Every token must have been consumed, except
    /// trailing trivia.
    pub(crate) fn finish(mut self) -> Parse {
        while let Some(token) = self.ahead.pop() {
            assert!(
                token.kind.is_trivia(),
                "the grammar left {:?} unconsumed",
                token.kind
            );
            self.consumed.push(token);
        }
        // Lexical errors came first; a stable sort keeps them before
        // syntax errors at the same place.
        self.errors.sort_by_key(|e| e.range.start());
        let green = build::build(self.text, &self.consumed, self.events);
        Parse {
            green,
            errors: self.errors,
        }
    }

    /// The `n`th significant token ahead, or [`SyntaxKind::Eof`].
    pub(crate) fn nth(&self, n: usize) -> SyntaxKind {
        self.nth_token(n).map_or(SyntaxKind::Eof, |t| t.kind)
    }

    pub(crate) fn current(&self) -> SyntaxKind {
        self.nth(0)
    }

    pub(crate) fn at(&self, kind: SyntaxKind) -> bool {
        self.current() == kind
    }

    /// The current token's text; empty at the end.
    pub(crate) fn current_text(&self) -> &'t str {
        &self.text[self.current_range()]
    }

    /// The current token's range; empty at the end of the text.
    pub(crate) fn current_range(&self) -> TextRange {
        self.nth_token(0)
            .map_or_else(|| TextRange::empty(TextSize::of(self.text)), |t| t.range)
    }

    fn nth_token(&self, n: usize) -> Option<&Token> {
        self.ahead
            .iter()
            .rev()
            .filter(|t| !t.kind.is_trivia())
            .nth(n)
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
        while let Some(token) = self.ahead.pop() {
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

    /// Reports a problem at the current token, or at the end.
    pub(crate) fn error(&mut self, kind: SyntaxErrorKind) {
        let range = self.current_range();
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
    /// reports [`SyntaxErrorKind::TooDeep`] and returns `None` without
    /// running `rule`; the caller recovers. Below it, the stack grows as
    /// needed, so depth is bounded by memory, not by the thread's stack.
    pub(crate) fn guarded<R>(&mut self, rule: impl FnOnce(&mut Parser<'t>) -> R) -> Option<R> {
        if self.depth >= DEPTH_LIMIT {
            self.error(SyntaxErrorKind::TooDeep);
            return None;
        }
        self.depth += 1;
        let result = stacker::maybe_grow(RED_ZONE, STACK_SEGMENT, || rule(self));
        self.depth -= 1;
        Some(result)
    }
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
