//! Turns parser events into a rowan tree, placing trivia on the way.
//!
//! Trivia is lazy: it waits until the next token or node needs a place.
//! Comments that start a line, after the last blank line before a node,
//! go inside that node, with the whitespace that follows them; so a
//! comment above a declaration belongs to it. All other trivia goes to
//! whichever node is open when the next token arrives, so trailing trivia
//! bubbles up to the parent instead of hanging off the end of a child.
//!
//! Nodes are built directly rather than through rowan's builder: its node
//! cache rehashes whole subtrees recursively, which overflows the stack on
//! deep trees. Tokens are still interned.

use std::collections::HashMap;

use rowan::{GreenNode, GreenToken, NodeOrToken};

use nomi_base::TextRange;

use super::Event;
use crate::SyntaxKind;
use crate::lexer::Token;

/// Panics unless the events place every token exactly once, in order, in a
/// single root: the tree's text is then the source text.
pub(super) fn build(text: &str, tokens: &[Token], mut events: Vec<Event>) -> GreenNode {
    let mut sink = Sink {
        text,
        tokens,
        next: 0,
        open: Vec::new(),
        children: Vec::new(),
        interned: HashMap::new(),
        root: None,
    };
    let mut parents = Vec::new();
    for i in 0..events.len() {
        match std::mem::replace(&mut events[i], Event::Tombstone) {
            Event::Start {
                kind,
                forward_parent,
            } => {
                // Collect the chain of nodes `precede` put around this one;
                // the outermost opens first.
                parents.push(kind);
                let mut at = i;
                let mut forward = forward_parent;
                while let Some(offset) = forward {
                    at += offset.get() as usize;
                    let Event::Start {
                        kind,
                        forward_parent,
                    } = std::mem::replace(&mut events[at], Event::Tombstone)
                    else {
                        unreachable!("a forward parent is a Start");
                    };
                    parents.push(kind);
                    forward = forward_parent;
                }
                for kind in parents.drain(..).rev() {
                    sink.start(kind);
                }
            }
            Event::Token { kind } => sink.token(kind),
            Event::Finish => sink.finish(),
            Event::Tombstone => {}
        }
    }
    assert!(sink.open.is_empty(), "every node must be finished");
    assert_eq!(sink.next, tokens.len(), "a lossless tree holds every token");
    let green = sink.root.expect("a tree has a root");
    assert_eq!(
        usize::from(green.text_len()),
        text.len(),
        "a lossless tree holds every byte"
    );
    green
}

struct Sink<'a> {
    text: &'a str,
    tokens: &'a [Token],
    /// The next token to place.
    next: usize,
    /// Nodes started and not finished, with where their children start.
    open: Vec<(SyntaxKind, usize)>,
    /// Children of the open nodes, innermost last.
    children: Vec<NodeOrToken<GreenNode, GreenToken>>,
    interned: HashMap<(SyntaxKind, &'a str), GreenToken>,
    root: Option<GreenNode>,
}

impl Sink<'_> {
    fn start(&mut self, kind: SyntaxKind) {
        let mut leading = 0;
        if self.open.is_empty() {
            assert!(self.root.is_none(), "a tree has one root");
        } else if kind != SyntaxKind::Missing {
            // An empty node has no comment to lead.
            let pending = self.pending_trivia();
            leading = self.leading_comments(pending);
            self.place(pending - leading);
        }
        self.open.push((kind, self.children.len()));
        // Placed now, so the outermost of nested nodes gets them.
        self.place(leading);
    }

    fn token(&mut self, kind: SyntaxKind) {
        let pending = self.pending_trivia();
        self.place(pending);
        let token = self
            .tokens
            .get(self.next)
            .expect("an event for every token, and a token for every event");
        assert!(!token.kind.is_trivia(), "token events are significant");
        self.push_token(kind, token.range);
        self.next += 1;
    }

    fn finish(&mut self) {
        if self.open.len() == 1 {
            // The root holds whatever trivia ends the file.
            self.place(self.tokens.len() - self.next);
        }
        let (kind, first) = self.open.pop().expect("a Finish for every Start");
        let node = GreenNode::new(kind.into(), self.children.drain(first..));
        if self.open.is_empty() {
            self.root = Some(node);
        } else {
            self.children.push(NodeOrToken::Node(node));
        }
    }

    /// How many trivia tokens wait before the next significant one.
    fn pending_trivia(&self) -> usize {
        self.tokens[self.next..]
            .iter()
            .take_while(|t| t.kind.is_trivia())
            .count()
    }

    /// How many of the `pending` trivia tokens lead the next node: from
    /// the first comment that starts a line after the last blank line.
    fn leading_comments(&self, pending: usize) -> usize {
        let trivia = &self.tokens[self.next..self.next + pending];
        let mut leading = 0;
        // Walk backwards: whitespace and newlines join a later comment,
        // a line-starting comment is a candidate, and a blank line or a
        // comment that follows code on its line ends the search.
        let mut newlines = 0;
        for i in (0..pending).rev() {
            match trivia[i].kind {
                SyntaxKind::Newline => {
                    newlines += 1;
                    if newlines == 2 {
                        break;
                    }
                }
                SyntaxKind::LineComment | SyntaxKind::BlockComment => {
                    if !self.starts_line(self.next + i) {
                        break;
                    }
                    newlines = 0;
                    leading = pending - i;
                }
                _ => {}
            }
        }
        leading
    }

    /// Whether only spaces lie between the token at `index` and the start
    /// of its line.
    fn starts_line(&self, index: usize) -> bool {
        self.tokens[..index]
            .iter()
            .rev()
            .find(|t| !matches!(t.kind, SyntaxKind::Whitespace | SyntaxKind::Tab))
            .is_none_or(|t| t.kind == SyntaxKind::Newline)
    }

    fn place(&mut self, count: usize) {
        for i in self.next..self.next + count {
            let token = self.tokens[i];
            debug_assert!(token.kind.is_trivia());
            self.push_token(token.kind, token.range);
        }
        self.next += count;
    }

    fn push_token(&mut self, kind: SyntaxKind, range: TextRange) {
        let text = &self.text[range];
        let token = self
            .interned
            .entry((kind, text))
            .or_insert_with(|| GreenToken::new(kind.into(), text))
            .clone();
        self.children.push(NodeOrToken::Token(token));
    }
}
