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
    /// Tokens consumed though not indented, which elm rejects and
    /// elm-format takes. Only the parser knows the indentation context, so
    /// it records them for [`crate::validate`].
    pub(crate) unindented: Vec<TextRange>,
    /// Indentation checks a token passed at [`FAR_COLUMN`] or further
    /// right, where a release that keeps columns in 16 bits sees another
    /// column.
    pub(crate) far: Vec<FarCheck>,
    /// How deeply grammar rules nested.
    deepest: u32,
}

/// Rule nesting a tree may reach and still drop on any thread's stack.
/// rowan drops a tree recursively, so a deeper one drops on a stack of
/// its own, or a 1 MiB main thread (Windows') overflows near 3,000.
const SHALLOW: u32 = 100;

impl Drop for Parse {
    fn drop(&mut self) {
        if self.deepest > SHALLOW {
            let empty = GreenNode::new(SyntaxKind::Error.into(), []);
            let green = std::mem::replace(&mut self.green, empty);
            stacker::grow(nomi_base::STACK_SIZE, move || drop(green));
        }
    }
}

/// The leftmost column, counting from 0, where a 16-bit 1-based column
/// wraps.
pub(crate) const FAR_COLUMN: u32 = u16::MAX as u32;

/// A token that passed an indentation check far right on its line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FarCheck {
    pub(crate) range: TextRange,
    pub(crate) column: u32,
    /// The column it had to reach: the enclosing construct's 1-based one.
    pub(crate) min_column: u32,
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

    /// The errors and the unindented tokens elm reports, in source order;
    /// for the grammar's tests.
    #[cfg(test)]
    pub(crate) fn elm_errors(&self) -> Vec<SyntaxError> {
        let mut errors = self.errors.clone();
        errors.extend(self.unindented.iter().map(|&range| SyntaxError {
            kind: SyntaxErrorKind::ExpectedIndent,
            range,
        }));
        errors.sort_by_key(|e| e.range.start());
        errors
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
    /// elm-format's legacy operator definition, as in `(=>) a b = …`.
    OperatorDefinition,
    /// elm-format's legacy range, as in `[1..3]`. elm wants `List.range`.
    ListRange,
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
    /// Elm 0.18's infix declaration, as in `infixl 0 |>`. elm wants
    /// `infix left 0 (|>) = apR`, and only in a kernel package.
    LegacyInfix,
    /// Elm 0.16's port definition, as in `port out = signal`. elm wants
    /// a port's type, as in `port out : String -> Cmd msg`.
    LegacyPort,
    /// `then` for `of` in `case x then`, which elm-format reads.
    CaseThen,
    /// `=>` for `->`, which elm-format reads.
    FatArrow,
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
    /// An infix precedence other than one decimal digit, such as `10`, or
    /// one joined to its associativity, as in `right0`, which elm-format
    /// takes, or to the operator after it, as in `infixl 7===`.
    BadPrecedence,
    /// An operator starting with `.` right after an operand that takes
    /// field accesses, as in `model.=count`: a field access gone wrong.
    JoinedDotOperator,
    /// A carriage return between tokens that no line feed follows.
    BareCarriageReturn,
    /// `..` as an operator, as in `exposing (a, (..))`, which elm takes.
    DotDotOperator,
    /// An operator starting with `.`, which elm reads as a field
    /// accessor, as in `40 .+ 2` or `f . g`.
    DotOperator,
    /// Text after a code block's last declaration or expression, which
    /// elm-format wants the block to end with.
    TrailingText,
    /// A `{- -}` comment opening a code block of expressions, which
    /// elm-format doesn't read as one.
    LeadingBlockComment,
    /// A shader block that elm's GLSL parser, language-glsl, rejects. The
    /// range is empty, where the parser stopped.
    ShaderProblem,
}

/// How deeply grammar rules may nest. Far beyond real code, which rarely
/// passes 50, but generated code can nest thousands deep.
/// It bounds the tree's depth, which matters past the parser: rowan drops
/// trees recursively, and [`nomi_base::STACK_SIZE`] is sized for the
/// deepest tree this admits.
pub const DEPTH_LIMIT: u32 = 25_000;

/// Stack kept free before a guarded rule runs, and the size of each new
/// segment when it isn't.
pub(crate) const RED_ZONE: usize = 128 * 1024;
pub(crate) const STACK_SEGMENT: usize = 2 * 1024 * 1024;

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
    /// The most `depth` has been.
    deepest: u32,
    /// Set once nesting passed [`DEPTH_LIMIT`]. The rest of the input is
    /// then one error node, and rules unwinding from there report nothing.
    gave_up: bool,
    /// The leftmost column a continuation token may start at: one past
    /// the enclosing construct's column, or 0 in a fragment.
    min_column: u32,
    /// Where a token starts that its block placed by alignment, such as a
    /// case branch's first token. It counts as indented.
    anchor: Option<TextSize>,
    layout: Layout,
    unindented: Vec<TextRange>,
    far: Vec<FarCheck>,
}

/// Whose reading of a module the parser follows: which continuations
/// must be indented to belong to the construct being parsed, and whether
/// a `"` string may span lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layout {
    /// Every one, as elm wants. Where a token isn't, the construct ends,
    /// which recovers well from unfinished code.
    Strict,
    /// Only arguments, as elm-format wants (`indented` in its
    /// `Parse/Helpers.hs`): a token any further rule must take, such as
    /// the body after `=`, may sit anywhere. A `"` string runs on past
    /// a line feed.
    Loose,
}

impl<'t> Parser<'t> {
    pub(crate) fn new(text: &'t str) -> Parser<'t> {
        Parser::with_layout(text, Layout::Strict)
    }

    pub(crate) fn with_layout(text: &'t str, layout: Layout) -> Parser<'t> {
        Parser {
            text,
            lexer: match layout {
                Layout::Strict => Lexer::new(text),
                Layout::Loose => Lexer::new(text).strings_span_lines(),
            },
            lexer_column: 0,
            ahead: VecDeque::new(),
            lexed: 0,
            consumed: Vec::new(),
            events: Vec::new(),
            errors: Vec::new(),
            depth: 0,
            deepest: 0,
            gave_up: false,
            min_column: 1,
            anchor: None,
            layout,
            unindented: Vec::new(),
            far: Vec::new(),
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
            far: self.far,
            deepest: self.deepest,
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

    /// The `n`th significant token's text, if there is one.
    pub(crate) fn nth_text(&mut self, n: usize) -> Option<&'t str> {
        self.nth_token(n).map(|a| &self.text[a.token.range])
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
    /// [`Parser::bump_symbol`] splits the operator. elm-format reads `=>`
    /// for `->` too; validation flags it for elm.
    pub(crate) fn nth_at_symbol(&mut self, n: usize, kind: SyntaxKind) -> bool {
        self.nth_token(n).is_some_and(|a| {
            let text = &self.text[a.token.range];
            a.token.kind == kind
                || (is_operator_token(a.token.kind)
                    && (kind.text().is_some_and(|symbol| text.starts_with(symbol))
                        || kind == SyntaxKind::Arrow && text.starts_with("=>")))
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
            self.split(kind);
        }
        self.bump();
    }

    /// Consumes the start of the current token as `kind`, whose fixed,
    /// ASCII text it spells, and lexes the rest again.
    pub(crate) fn bump_prefix_as(&mut self, kind: SyntaxKind) {
        let text = self.current_text();
        debug_assert!(kind.text().is_some_and(|t| text.starts_with(t)));
        if kind.text() != Some(text) {
            self.split(kind);
        }
        self.bump_as(kind);
    }

    fn split(&mut self, kind: SyntaxKind) {
        let (i, a) = self.nth_ahead(0).expect("at a token");
        let len = TextSize::of(kind.text().expect("a fixed text"));
        let cut = a.token.range.start() + len;
        self.ahead.truncate(i + 1);
        self.ahead[i].token = Token {
            kind,
            range: TextRange::new(a.token.range.start(), cut),
        };
        self.lexer.rewind(cut, a.lex_errors as usize);
        // The text is ASCII, so bytes are columns.
        self.lexer_column = a.column + u32::from(len);
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

    /// Whether the current token continues the construct being parsed:
    /// it is indented enough, or is the anchor, or the layout is
    /// [`Layout::Loose`]. The end never does.
    pub(crate) fn at_indented(&mut self) -> bool {
        let indented = self.at_indented_strictly();
        indented || (self.layout == Layout::Loose && !self.at(SyntaxKind::Eof))
    }

    /// Whether the current token is indented enough to continue the
    /// construct being parsed, or is the anchor, whatever the layout:
    /// before an argument, and where recovery decides what to skip. The
    /// end never is.
    pub(crate) fn at_indented_strictly(&mut self) -> bool {
        let Some(a) = self.nth_token(0) else {
            return false;
        };
        if self.anchor == Some(a.token.range.start()) {
            return true;
        }
        let indented = a.column >= self.min_column;
        if indented && a.column >= FAR_COLUMN {
            let check = FarCheck {
                range: a.token.range,
                column: a.column,
                min_column: self.min_column,
            };
            if self.far.last() != Some(&check) {
                self.far.push(check);
            }
        }
        indented
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

    /// Consumes the current token as its own kind.
    pub(crate) fn bump(&mut self) {
        let kind = self.current();
        self.bump_as(kind);
    }

    /// Consumes the current token as `kind`, such as `alias` as
    /// [`SyntaxKind::AliasKw`].
    pub(crate) fn bump_as(&mut self, kind: SyntaxKind) {
        assert!(!self.at(SyntaxKind::Eof), "bumped past the end");
        if let Some(a) = self.nth_token(0)
            && a.column < self.min_column
            && self.anchor != Some(a.token.range.start())
        {
            self.unindented.push(a.token.range);
        }
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
    /// Reports the current token as indented too little by a rule of its
    /// construct's own, as `let` definitions must be right of the `let`:
    /// an error in the strict layout, and in the loose one, which
    /// elm-format reads, a token elm rejects.
    pub(crate) fn misindented(&mut self) {
        match self.layout {
            Layout::Strict => self.error(SyntaxErrorKind::ExpectedIndent),
            Layout::Loose => {
                if let Some(a) = self.nth_token(0) {
                    self.unindented.push(a.token.range);
                }
            }
        }
    }

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
        self.deepest = self.deepest.max(self.depth);
        let result = stacker::maybe_grow(RED_ZONE, STACK_SEGMENT, || rule(self));
        self.depth -= 1;
        Some(result)
    }
}

/// The column after `text`, starting at `column`. Columns count
/// characters, as every elm release does, ignoring carriage returns as
/// elm's `eatSpaces` does.
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
