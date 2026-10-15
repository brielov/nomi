//! Splits source text into tokens. Lossless: every byte belongs to exactly
//! one token, so the tokens' texts concatenate back to the input. Malformed
//! input never stops the lexer; it keeps the token and reports a
//! [`LexError`].

use nomi_base::{TextRange, TextSize, ghc};

use crate::SyntaxKind;

/// A token: its kind and the text it spans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: SyntaxKind,
    pub range: TextRange,
}

/// A problem the lexer found, and where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LexError {
    pub kind: LexErrorKind,
    pub range: TextRange,
}

/// What the lexer found wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LexErrorKind {
    /// A tab, which Elm doesn't take as whitespace.
    Tab,
    /// A NUL byte inside a comment.
    UnexpectedEncoding,
    /// A `{-` comment the text ends inside.
    UnterminatedBlockComment,
    /// `1.` with no digit after the dot.
    MissingFractionDigits,
    /// An exponent without digits, as in `1e`.
    MissingExponentDigits,
    /// `0x` without hexadecimal digits, or with a letter after them.
    InvalidHexDigit,
    /// A decimal number starting with `0`, as in `012`.
    LeadingZero,
    /// A number joined to a name after it, as in `1x`.
    NumberFollowedByIdentifier,
    /// A character literal the line or the text ends inside.
    UnterminatedCharacter,
    /// A character literal holding other than one character.
    CharacterLength,
    /// A backslash escape neither tool knows, as in `\q`.
    UnknownEscape,
    /// `\u` without braces around its digits.
    InvalidUnicodeEscapeFormat,
    /// `\u{}` without digits, or past U+10FFFF.
    InvalidUnicodeCodePoint,
    /// `\u{…}` with fewer than 4 or more than 6 digits.
    InvalidUnicodeEscapeLength,
    /// A `"` string left open.
    UnterminatedString,
    /// A `"""` string the text ends inside.
    UnterminatedMultilineString,
    /// A `[glsl|` block the text ends inside.
    UnterminatedShaderBlock,
    /// A character that starts no token.
    InvalidToken,
}

/// The tokens of a text, without an end-of-input token, and the problems
/// found on the way, in source order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lexed {
    tokens: Vec<Token>,
    errors: Vec<LexError>,
}

impl Lexed {
    /// The tokens, in order.
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    /// The problems, in source order.
    pub fn errors(&self) -> &[LexError] {
        &self.errors
    }
}

/// Lexes `text`. Panics if it is 4 GiB or longer.
///
/// Malformed literals keep their literal kind, so the parser can still
/// place them; only text that starts no token becomes
/// [`SyntaxKind::ErrorToken`]. Contextual keywords lex as
/// [`SyntaxKind::LowerIdent`], and splits such as `=-` are the parser's.
pub fn lex(text: &str) -> Lexed {
    let mut lexer = Lexer::new(text);
    let mut tokens = Vec::new();
    while let Some(token) = lexer.next_token() {
        tokens.push(token);
    }
    Lexed {
        tokens,
        errors: lexer.errors,
    }
}

/// Lexes one token at a time, so the parser can split a token and lex
/// again from inside it.
pub(crate) struct Lexer<'a> {
    text: &'a str,
    bytes: &'a [u8],
    offset: usize,
    errors: Vec<LexError>,
    /// Whether a `"` string may take in a line feed, as elm-format's do
    /// (`Parse/Literal.hs`); elm ends it there unterminated.
    strings_span_lines: bool,
}

impl<'a> Lexer<'a> {
    /// Panics if `text` is 4 GiB or longer.
    pub(crate) fn new(text: &'a str) -> Lexer<'a> {
        assert!(
            u32::try_from(text.len()).is_ok(),
            "source text must be shorter than 4 GiB"
        );
        Lexer {
            text,
            bytes: text.as_bytes(),
            offset: 0,
            errors: Vec::new(),
            strings_span_lines: false,
        }
    }

    /// Lets `"` strings span lines.
    pub(crate) fn strings_span_lines(mut self) -> Lexer<'a> {
        self.strings_span_lines = true;
        self
    }

    /// The next token, or `None` at the end.
    pub(crate) fn next_token(&mut self) -> Option<Token> {
        (self.offset < self.bytes.len()).then(|| self.token())
    }

    /// The problems found so far, in source order.
    pub(crate) fn errors(&self) -> &[LexError] {
        &self.errors
    }

    pub(crate) fn into_errors(self) -> Vec<LexError> {
        self.errors
    }

    /// Lexes again from `offset`, a character boundary, forgetting all but
    /// the first `errors` problems.
    pub(crate) fn rewind(&mut self, offset: TextSize, errors: usize) {
        self.offset = offset.into();
        self.errors.truncate(errors);
    }

    fn token(&mut self) -> Token {
        let start = self.offset;
        let kind = match self.bytes[start] {
            b'\n' => self.single(SyntaxKind::Newline),
            b'\r' if self.byte_at(start + 1) == Some(b'\n') => {
                self.offset += 2;
                SyntaxKind::Newline
            }
            // Elm skips a bare carriage return like a space.
            b' ' | b'\r' => {
                while match self.byte_at(self.offset) {
                    Some(b' ') => true,
                    Some(b'\r') => self.byte_at(self.offset + 1) != Some(b'\n'),
                    _ => false,
                } {
                    self.offset += 1;
                }
                SyntaxKind::Whitespace
            }
            b'\t' => {
                self.offset += 1;
                self.error(LexErrorKind::Tab, start, self.offset);
                SyntaxKind::Tab
            }
            b'-' if self.byte_at(start + 1) == Some(b'-') => {
                self.line_comment();
                SyntaxKind::LineComment
            }
            // Elm's space parser needs a byte after `{-` to see a comment.
            b'{' if self.byte_at(start + 1) == Some(b'-') && self.byte_at(start + 2).is_some() => {
                self.block_comment()
            }
            b'[' if self.starts_with(start, b"[glsl|") => {
                self.shader_block();
                SyntaxKind::Glsl
            }
            b'"' => self.string(),
            b'\'' => {
                self.character();
                SyntaxKind::Char
            }
            b'0'..=b'9' => self.number(),
            b'(' => self.single(SyntaxKind::LParen),
            b')' => self.single(SyntaxKind::RParen),
            b'[' => self.single(SyntaxKind::LBracket),
            b']' => self.single(SyntaxKind::RBracket),
            b'{' => self.single(SyntaxKind::LBrace),
            b'}' => self.single(SyntaxKind::RBrace),
            b',' => self.single(SyntaxKind::Comma),
            b'\\' => self.single(SyntaxKind::Backslash),
            b'_' => self.single(SyntaxKind::Underscore),
            b'`' => match self.backticked_end(start) {
                // An infix call, an operator to the grammar; elm's
                // validation reports it.
                Some(end) => {
                    self.offset = end;
                    SyntaxKind::Operator
                }
                None => self.identifier_or_invalid(),
            },
            byte if is_operator(byte) => self.operator(),
            _ if self.char_at(start).is_some_and(is_operator_char) => self.operator(),
            _ => self.identifier_or_invalid(),
        };
        debug_assert!(self.offset > start, "the lexer must always make progress");
        Token {
            kind,
            range: range(start, self.offset),
        }
    }

    fn single(&mut self, kind: SyntaxKind) -> SyntaxKind {
        self.offset += 1;
        kind
    }

    fn line_comment(&mut self) {
        self.offset += 2;
        while let Some(byte) = self.byte_at(self.offset) {
            match byte {
                b'\n' => break,
                b'\r' if self.byte_at(self.offset + 1) == Some(b'\n') => break,
                0 => {
                    self.error(LexErrorKind::UnexpectedEncoding, self.offset, self.offset);
                    self.offset += 1;
                }
                _ => self.offset += self.scalar_width(self.offset),
            }
        }
    }

    fn block_comment(&mut self) -> SyntaxKind {
        let start = self.offset;
        let kind = if self.byte_at(start + 2) == Some(b'|') {
            self.offset += 3;
            SyntaxKind::DocComment
        } else {
            self.offset += 2;
            SyntaxKind::BlockComment
        };
        let mut depth = 1_u32;
        while self.offset < self.bytes.len() {
            if self.starts_with(self.offset, b"{-") {
                depth += 1;
                self.offset += 2;
            } else if self.starts_with(self.offset, b"-}") {
                depth -= 1;
                self.offset += 2;
                if depth == 0 {
                    return kind;
                }
            } else {
                // elm also rejects a tab here, which elm-format takes;
                // validation reports it.
                if self.bytes[self.offset] == 0 {
                    self.error(LexErrorKind::UnexpectedEncoding, self.offset, self.offset);
                }
                self.offset += self.scalar_width(self.offset);
            }
        }
        self.error(LexErrorKind::UnterminatedBlockComment, start, self.offset);
        kind
    }

    fn shader_block(&mut self) {
        let start = self.offset;
        self.offset += b"[glsl|".len();
        while self.offset < self.bytes.len() && !self.starts_with(self.offset, b"|]") {
            self.offset += self.scalar_width(self.offset);
        }
        if self.offset == self.bytes.len() {
            self.error(LexErrorKind::UnterminatedShaderBlock, start, self.offset);
        } else {
            self.offset += 2;
        }
    }

    fn string(&mut self) -> SyntaxKind {
        let start = self.offset;
        if self.starts_with(start, b"\"\"\"") {
            self.offset += 3;
            self.quoted(start, Quote::Multiline);
            SyntaxKind::MultilineString
        } else {
            self.offset += 1;
            self.quoted(start, Quote::Single);
            SyntaxKind::String
        }
    }

    fn quoted(&mut self, start: usize, quote: Quote) {
        while self.offset < self.bytes.len() {
            if self.starts_with(self.offset, quote.delimiter()) {
                self.offset += quote.delimiter().len();
                return;
            }
            match self.bytes[self.offset] {
                b'\\' => self.escape(),
                b'\n' if quote == Quote::Single && !self.strings_span_lines => break,
                _ => self.offset += self.scalar_width(self.offset),
            }
        }
        self.error(quote.unterminated(), start, self.offset);
    }

    fn character(&mut self) {
        let start = self.offset;
        self.offset += 1;
        let mut characters = 0_u32;
        while let Some(byte) = self.byte_at(self.offset) {
            match byte {
                b'\'' => {
                    self.offset += 1;
                    if characters != 1 {
                        self.error(LexErrorKind::CharacterLength, start, self.offset);
                    }
                    return;
                }
                b'\n' => break,
                b'\\' => {
                    self.escape();
                    characters += 1;
                }
                _ => {
                    self.offset += self.scalar_width(self.offset);
                    characters += 1;
                }
            }
        }
        self.error(LexErrorKind::UnterminatedCharacter, start, self.offset);
    }

    fn escape(&mut self) {
        let start = self.offset;
        self.offset += 1;
        let Some(byte) = self.byte_at(self.offset) else {
            // The enclosing literal reports an escape cut off by the end.
            return;
        };
        match byte {
            b'n' | b'r' | b't' | b'"' | b'\'' | b'\\' => self.offset += 1,
            b'u' => self.unicode_escape(start),
            // elm-format's Haskell-style `\x41`; validation reports it
            // for elm, and the characters elm-format can't hold.
            b'x' => {
                self.offset += 1;
                self.consume_while(|byte| byte.is_ascii_hexdigit());
            }
            _ => {
                self.offset += self.scalar_width(self.offset);
                self.error(LexErrorKind::UnknownEscape, start, self.offset);
            }
        }
    }

    fn unicode_escape(&mut self, start: usize) {
        self.offset += 1;
        if self.byte_at(self.offset) != Some(b'{') {
            self.error(LexErrorKind::InvalidUnicodeEscapeFormat, start, self.offset);
            return;
        }
        self.offset += 1;
        let digits_start = self.offset;
        let mut value = 0_u32;
        while let Some(digit) = self
            .byte_at(self.offset)
            .and_then(|b| char::from(b).to_digit(16))
        {
            value = value.saturating_mul(16).saturating_add(digit);
            self.offset += 1;
        }
        let digits = self.offset - digits_start;
        let closed = self.byte_at(self.offset) == Some(b'}');
        if closed {
            self.offset += 1;
        }
        // Like upstream, surrogates are accepted; only the range is checked.
        let problem = if !closed {
            Some(LexErrorKind::InvalidUnicodeEscapeFormat)
        } else if digits == 0 || value > 0x10_FFFF {
            Some(LexErrorKind::InvalidUnicodeCodePoint)
        } else if !(4..=6).contains(&digits) {
            Some(LexErrorKind::InvalidUnicodeEscapeLength)
        } else {
            None
        };
        if let Some(problem) = problem {
            self.error(problem, start, self.offset);
        }
    }

    /// Follows upstream's number chompers: where they stop, the number
    /// ends, and what follows lexes on its own.
    fn number(&mut self) -> SyntaxKind {
        let start = self.offset;
        if self.starts_with(start, b"0x") {
            self.offset += 2;
            let digits_start = self.offset;
            self.consume_while(|byte| byte.is_ascii_hexdigit());
            if self.offset == digits_start {
                self.error(LexErrorKind::InvalidHexDigit, self.offset, self.offset);
            } else {
                self.check_number_end(LexErrorKind::InvalidHexDigit);
            }
            return SyntaxKind::Int;
        }
        if self.bytes[start] == b'0' {
            self.offset += 1;
            match self.byte_at(self.offset) {
                Some(byte) if byte.is_ascii_digit() => {
                    self.error(LexErrorKind::LeadingZero, self.offset, self.offset);
                    return SyntaxKind::Int;
                }
                Some(b'.') => return self.fraction(),
                _ => {
                    self.check_number_end(LexErrorKind::NumberFollowedByIdentifier);
                    return SyntaxKind::Int;
                }
            }
        }
        self.consume_while(|byte| byte.is_ascii_digit());
        match self.byte_at(self.offset) {
            Some(b'.') => self.fraction(),
            Some(b'e' | b'E') => {
                self.exponent();
                SyntaxKind::Float
            }
            _ => {
                self.check_number_end(LexErrorKind::NumberFollowedByIdentifier);
                SyntaxKind::Int
            }
        }
    }

    /// At the `.` after the integer part.
    fn fraction(&mut self) -> SyntaxKind {
        // elm-format's `[1..3]`, a range; the grammar reports it for elm.
        if self.byte_at(self.offset + 1) == Some(b'.') {
            return SyntaxKind::Int;
        }
        if !self
            .byte_at(self.offset + 1)
            .is_some_and(|byte| byte.is_ascii_digit())
        {
            self.error(
                LexErrorKind::MissingFractionDigits,
                self.offset,
                self.offset,
            );
            return SyntaxKind::Int;
        }
        self.offset += 1;
        self.consume_while(|byte| byte.is_ascii_digit());
        if matches!(self.byte_at(self.offset), Some(b'e' | b'E')) {
            self.exponent();
        } else {
            self.check_number_end(LexErrorKind::NumberFollowedByIdentifier);
        }
        SyntaxKind::Float
    }

    fn exponent(&mut self) {
        self.offset += 1;
        let exponent_start = self.offset;
        if matches!(self.byte_at(self.offset), Some(b'+' | b'-')) {
            self.offset += 1;
        }
        let digits_start = self.offset;
        self.consume_while(|byte| byte.is_ascii_digit());
        if self.offset == digits_start {
            self.error(
                LexErrorKind::MissingExponentDigits,
                exponent_start,
                exponent_start,
            );
        }
    }

    fn check_number_end(&mut self, kind: LexErrorKind) {
        if self
            .char_at(self.offset)
            .is_some_and(is_identifier_continue)
        {
            self.error(kind, self.offset, self.offset);
        }
    }

    fn operator(&mut self) -> SyntaxKind {
        let start = self.offset;
        self.offset = self.chars_end(start, is_operator_char);
        // `-`, `=`, `..` and the like have kinds of their own, and
        // elm-format reads a lone `→` as `->`.
        match &self.text[start..self.offset] {
            "→" => SyntaxKind::Arrow,
            text => SyntaxKind::from_text(text).unwrap_or(SyntaxKind::Operator),
        }
    }

    /// Where elm-format's backticked function, `` `f` `` or `` `M.f` ``,
    /// starting at `start` ends, if one does.
    fn backticked_end(&self, start: usize) -> Option<usize> {
        let mut end = start + 1;
        loop {
            let segment = end;
            let kind = identifier_kind(self.char_at(end)?)?;
            end = self.chars_end(end, is_identifier_continue);
            match (kind, self.byte_at(end)?) {
                (SyntaxKind::UpperIdent, b'.') => end += 1,
                (SyntaxKind::LowerIdent, b'`')
                    if SyntaxKind::from_text(&self.text[segment..end]).is_none() =>
                {
                    return Some(end + 1);
                }
                _ => return None,
            }
        }
    }

    fn identifier_or_invalid(&mut self) -> SyntaxKind {
        let start = self.offset;
        let first = self
            .char_at(start)
            .expect("the lexer stops only at character boundaries");
        self.offset += first.len_utf8();
        let Some(kind) = identifier_kind(first) else {
            self.error(LexErrorKind::InvalidToken, start, self.offset);
            return SyntaxKind::ErrorToken;
        };
        self.offset = self.chars_end(self.offset, is_identifier_continue);
        if kind == SyntaxKind::LowerIdent
            && let Some(keyword) = SyntaxKind::from_text(&self.text[start..self.offset])
            && keyword.is_keyword()
        {
            return keyword;
        }
        kind
    }

    fn consume_while(&mut self, predicate: impl Fn(u8) -> bool) {
        while self.byte_at(self.offset).is_some_and(&predicate) {
            self.offset += 1;
        }
    }

    /// The end of the characters from `from` on that `predicate` takes.
    fn chars_end(&self, mut from: usize, predicate: impl Fn(char) -> bool) -> usize {
        while let Some(c) = self.char_at(from).filter(|&c| predicate(c)) {
            from += c.len_utf8();
        }
        from
    }

    fn byte_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(offset).copied()
    }

    fn char_at(&self, offset: usize) -> Option<char> {
        self.text.get(offset..)?.chars().next()
    }

    fn scalar_width(&self, offset: usize) -> usize {
        self.char_at(offset)
            .expect("the lexer stops only at character boundaries")
            .len_utf8()
    }

    fn starts_with(&self, offset: usize, prefix: &[u8]) -> bool {
        self.bytes
            .get(offset..)
            .is_some_and(|rest| rest.starts_with(prefix))
    }

    fn error(&mut self, kind: LexErrorKind, start: usize, end: usize) {
        self.errors.push(LexError {
            kind,
            range: range(start, end),
        });
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    Single,
    Multiline,
}

impl Quote {
    fn delimiter(self) -> &'static [u8] {
        match self {
            Quote::Single => b"\"",
            Quote::Multiline => b"\"\"\"",
        }
    }

    fn unterminated(self) -> LexErrorKind {
        match self {
            Quote::Single => LexErrorKind::UnterminatedString,
            Quote::Multiline => LexErrorKind::UnterminatedMultilineString,
        }
    }
}

/// `lex` checked that every offset fits.
fn range(start: usize, end: usize) -> TextRange {
    let size = |n: usize| TextSize::new(n as u32);
    TextRange::new(size(start), size(end))
}

fn identifier_kind(c: char) -> Option<SyntaxKind> {
    if c.is_ascii_lowercase() {
        return Some(SyntaxKind::LowerIdent);
    }
    if c.is_ascii_uppercase() {
        return Some(SyntaxKind::UpperIdent);
    }
    if c.is_ascii() {
        return None;
    }
    // Rust's case properties are broader than GHC's: they call `ª`
    // lowercase and `Ⅷ` uppercase, and Elm accepts neither.
    if ghc::is_lower(c) {
        Some(SyntaxKind::LowerIdent)
    } else if ghc::is_upper(c) {
        Some(SyntaxKind::UpperIdent)
    } else {
        None
    }
}

fn is_identifier_continue(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphanumeric() || c == '_';
    }
    ghc::is_alpha(c)
}

/// Whether elm-format takes `c` in an operator: elm's characters, a few
/// more ASCII ones, and Haskell's `isSymbol`. elm's validation reports
/// the rest.
fn is_operator_char(c: char) -> bool {
    if c.is_ascii() {
        return is_operator(c as u8);
    }
    ghc::is_symbol(c)
}

/// Whether elm-format takes the ASCII `byte` in an operator: elm's
/// [`BINOP_CHARS`](nomi_base::diagnostic::code::BINOP_CHARS), and `$`, `#`,
/// `@` and `~`.
pub(crate) fn is_operator(byte: u8) -> bool {
    matches!(
        byte,
        b'+' | b'-'
            | b'/'
            | b'*'
            | b'='
            | b'.'
            | b'<'
            | b'>'
            | b':'
            | b'&'
            | b'|'
            | b'^'
            | b'?'
            | b'%'
            | b'!'
            | b'$'
            | b'#'
            | b'@'
            | b'~'
    )
}
