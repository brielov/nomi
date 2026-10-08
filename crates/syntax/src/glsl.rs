//! The GLSL parser elm runs on every shader block (`Parse/Shader.hs`):
//! language-glsl 0.3.0's `Language/GLSL/Parser.hs`, as a recognizer.
//!
//! Upstream is a Parsec parser, and what it accepts follows from Parsec's
//! rules as much as from the grammar: an alternative runs only when the
//! one before failed without consuming input, and only `try` rewinds. So
//! each rule here mirrors one of upstream's on the same rules, quirks
//! included: `x <<= 1` never parses, since `<` takes the first character
//! and leaves `<=` where an operand should be, and an `int` parameter
//! reads as `in` and a type named `t`.
//!
//! Whether a rule consumed input shows in the position: rules fail
//! without moving it, unless they consumed. Errors follow Parsec's too,
//! messages and all (`error.rs`), so the reported offset is where elm
//! reports the problem and the message is the one it quotes.

mod error;
mod words;

use icu_properties::GeneralCategory;

use self::error::{Error, Kind, Text};
use self::words::{KEYWORDS, RESERVED_WORDS, TYPES};
use crate::lexer::GENERAL_CATEGORY;
use crate::parser::{DEPTH_LIMIT, RED_ZONE, STACK_SEGMENT};

/// Why a shader doesn't parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Rejection {
    /// Upstream fails at this byte offset, with Parsec's message as elm
    /// shows it.
    At { offset: usize, message: String },
    /// Nesting passed [`DEPTH_LIMIT`] at this offset. Upstream has no
    /// limit.
    TooDeep(usize),
}

/// Checks a shader block's text, between `[glsl|` and `|]`.
pub(crate) fn check(text: &str) -> Result<(), Rejection> {
    let mut p = Glsl {
        text,
        pos: 0,
        error: Error::unknown(0),
        depth: 0,
        too_deep: None,
    };
    let result = (|| {
        p.blanks();
        p.many1(Glsl::external_declaration)?;
        p.eof()
    })();
    match (p.too_deep, result) {
        (Some(at), _) => Err(Rejection::TooDeep(at)),
        (None, Ok(())) => Ok(()),
        (None, Err(Fail)) => Err(Rejection::At {
            offset: p.error.offset,
            message: p.error.show(),
        }),
    }
}

/// A rule failed. Whether it consumed input shows in the position.
struct Fail;

type P<T = ()> = Result<T, Fail>;

struct Glsl<'t> {
    text: &'t str,
    pos: usize,
    /// The error of the input consumed so far, as Parsec carries it: a
    /// consuming rule replaces it, a failing one merges into it.
    error: Error,
    depth: u32,
    too_deep: Option<usize>,
}

type Rule<'r, 't, T = ()> = &'r dyn Fn(&mut Glsl<'t>) -> P<T>;

/// An infix operator of `buildExpressionParser`'s tables.
#[derive(Clone, Copy)]
enum Op {
    /// `infixLeft` and `infixRight`: the characters.
    Plain(&'static str),
    /// `infixLeft'`: not followed by `=`.
    NoEq(&'static str),
    /// `infixLeft''`: not followed by itself or `=`.
    Single(char),
}

/// `conditionalTable`, tightest first.
const CONDITIONAL: &[&[Op]] = {
    use Op::*;
    &[
        &[NoEq("*"), NoEq("/"), NoEq("%")],
        &[NoEq("+"), NoEq("-")],
        &[NoEq("<<"), NoEq(">>")],
        &[NoEq("<"), NoEq(">"), Plain("<="), Plain(">=")],
        &[Plain("=="), Plain("!=")],
        &[Single('&')],
        &[NoEq("^")],
        &[Single('|')],
        &[Plain("&&")],
        &[Plain("||")],
    ]
};

/// `assignmentTable`, a level for each operator.
const ASSIGNMENT: &[&[Op]] = {
    use Op::*;
    &[
        &[Plain("=")],
        &[Plain("+=")],
        &[Plain("-=")],
        &[Plain("*=")],
        &[Plain("/=")],
        &[Plain("%=")],
        &[Plain("<<=")],
        &[Plain(">>=")],
        &[Plain("&=")],
        &[Plain("^=")],
        &[Plain("|=")],
    ]
};

/// GHC's `isAlpha`.
fn is_letter(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphabetic();
    }
    matches!(
        GENERAL_CATEGORY.get(c),
        GeneralCategory::UppercaseLetter
            | GeneralCategory::LowercaseLetter
            | GeneralCategory::TitlecaseLetter
            | GeneralCategory::ModifierLetter
            | GeneralCategory::OtherLetter
    )
}

/// GHC's `isAlphaNum`: letters and numbers of any kind.
fn is_alpha_num(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphanumeric();
    }
    is_letter(c)
        || matches!(
            GENERAL_CATEGORY.get(c),
            GeneralCategory::DecimalNumber
                | GeneralCategory::LetterNumber
                | GeneralCategory::OtherNumber
        )
}

/// GHC's `isSpace`.
fn is_space(c: char) -> bool {
    match c {
        ' ' | '\t'..='\r' | '\u{a0}' => true,
        _ if (c as u32) <= 0x377 => false,
        _ => GENERAL_CATEGORY.get(c) == GeneralCategory::SpaceSeparator,
    }
}

impl<'t> Glsl<'t> {
    // Parsec's primitives and combinators.

    fn peek(&self) -> Option<char> {
        self.text[self.pos..].chars().next()
    }

    /// Fails with a message at `offset`, merged into the error.
    fn fail_at(&mut self, offset: usize, kind: Kind, text: Text) -> Fail {
        self.error.merge(Error::new(offset, kind, text));
        Fail
    }

    /// Consumes `len` bytes, which starts the error afresh.
    fn advance(&mut self, len: usize) {
        self.pos += len;
        self.error = Error::unknown(self.pos);
    }

    /// What a token parser met here: `show [c]`, or nothing at the end.
    fn met(&self, offset: usize) -> Text {
        match self.text[offset..].chars().next() {
            Some(c) => Text::ShowString(c),
            None => Text::Str(""),
        }
    }

    fn satisfy(&mut self, f: impl Fn(char) -> bool) -> P<char> {
        match self.peek() {
            Some(c) if f(c) => {
                self.advance(c.len_utf8());
                Ok(c)
            }
            _ => Err(self.fail_at(self.pos, Kind::SysUnExpect, self.met(self.pos))),
        }
    }

    /// `char`, which labels itself with its `show`.
    fn char(&mut self, c: char) -> P<char> {
        self.label(Text::ShowString(c), |p| p.satisfy(|d| d == c))
    }

    /// `string`: consumes what matches, so a partial match fails having
    /// consumed input, with the error where it started.
    fn string(&mut self, s: &'static str) -> P {
        let start = self.pos;
        let rest = &self.text[start..];
        if rest.starts_with(s) {
            self.advance(s.len());
            return Ok(());
        }
        let matched: usize = (rest.chars().zip(s.chars()))
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| a.len_utf8())
            .sum();
        let mut error = Error::new(start, Kind::SysUnExpect, self.met(start + matched));
        error.expect(Text::Show(s));
        if matched == 0 {
            self.error.merge(error);
        } else {
            self.pos += matched;
            self.error = error;
        }
        Err(Fail)
    }

    /// `eof`: `notFollowedBy anyToken <?> "end of input"`, which leaves
    /// its error even when it succeeds.
    fn eof(&mut self) -> P {
        let mut error = match self.peek() {
            None => Error::new(self.pos, Kind::SysUnExpect, Text::Str("")),
            Some(c) => Error::new(self.pos, Kind::UnExpect, Text::ShowChar(c)),
        };
        error.expect(Text::Str("end of input"));
        self.error.merge(error);
        if self.pos == self.text.len() {
            Ok(())
        } else {
            Err(Fail)
        }
    }

    /// `<?>`: when `rule` consumes nothing, `label` replaces what its
    /// error expects.
    fn label<T>(&mut self, label: Text, rule: impl FnOnce(&mut Self) -> P<T>) -> P<T> {
        let start = self.pos;
        let outer = std::mem::replace(&mut self.error, Error::unknown(start));
        let result = rule(self);
        if self.pos == start {
            if result.is_err() || self.error.is_known() {
                self.error.expect(label);
            }
            let inner = std::mem::replace(&mut self.error, outer);
            self.error.merge(inner);
        }
        result
    }

    /// `try`: a failure consumes nothing.
    fn attempt<T>(&mut self, rule: impl FnOnce(&mut Self) -> P<T>) -> P<T> {
        let start = self.pos;
        let outer = std::mem::replace(&mut self.error, Error::unknown(start));
        let result = rule(self);
        if result.is_err() {
            self.pos = start;
        }
        if self.pos == start {
            let inner = std::mem::replace(&mut self.error, outer);
            self.error.merge(inner);
        }
        result
    }

    /// `<|>` over `rules`: each runs only if those before failed without
    /// consuming.
    fn choice<T>(&mut self, rules: &[Rule<'_, 't, T>]) -> P<T> {
        let start = self.pos;
        for rule in rules {
            match rule(self) {
                Err(Fail) if self.pos == start => {}
                result => return result,
            }
        }
        Err(Fail)
    }

    /// `optionMaybe`: whether `rule` ran.
    fn optional(&mut self, rule: impl FnOnce(&mut Self) -> P) -> P<bool> {
        let start = self.pos;
        match rule(self) {
            Ok(()) => Ok(true),
            Err(Fail) if self.pos == start => Ok(false),
            Err(Fail) => Err(Fail),
        }
    }

    /// `many`. Each repetition after the first starts with a fresh error,
    /// as Parsec's does.
    fn many(&mut self, rule: impl Fn(&mut Self) -> P) -> P {
        let mut first = true;
        loop {
            let start = self.pos;
            if !first {
                self.error = Error::unknown(start);
            }
            match rule(self) {
                Ok(()) if self.pos == start => return Ok(()),
                Ok(()) => first = false,
                Err(Fail) if self.pos == start => return Ok(()),
                Err(Fail) => return Err(Fail),
            }
        }
    }

    fn many1(&mut self, rule: impl Fn(&mut Self) -> P) -> P {
        rule(self)?;
        self.many(rule)
    }

    fn sep_by(&mut self, rule: impl Fn(&mut Self) -> P, sep: impl Fn(&mut Self) -> P) -> P {
        if self.optional(&rule)? {
            self.many(|p| {
                sep(p)?;
                rule(p)
            })?;
        }
        Ok(())
    }

    /// `notFollowedBy`, which never consumes: it fails where `rule`
    /// stopped, with `unexpected (show c)` for what `rule` returned.
    fn not_followed_by(&mut self, rule: impl FnOnce(&mut Self) -> P<char>) -> P {
        let start = self.pos;
        let outer = std::mem::replace(&mut self.error, Error::unknown(start));
        let result = rule(self);
        if let Ok(c) = result {
            self.fail_at(self.pos, Kind::UnExpect, Text::ShowChar(c));
        }
        self.pos = start;
        let inner = std::mem::replace(&mut self.error, outer);
        self.error.merge(inner);
        match result {
            Ok(_) => Err(Fail),
            Err(Fail) => Ok(()),
        }
    }

    /// `manyTill anyChar end`.
    fn skip_till(&mut self, end: impl Fn(&mut Self) -> P) -> P {
        loop {
            let start = self.pos;
            match end(self) {
                Ok(()) => return Ok(()),
                Err(Fail) if self.pos != start => return Err(Fail),
                Err(Fail) => {}
            }
            self.satisfy(|_| true)?;
        }
    }

    /// Runs a rule that may recurse, growing the stack as needed.
    fn guarded(&mut self, rule: impl FnOnce(&mut Self) -> P) -> P {
        if self.depth >= DEPTH_LIMIT {
            self.too_deep.get_or_insert(self.pos);
            // Consumed, so nothing backtracks over it.
            self.pos = self.text.len();
            return Err(Fail);
        }
        self.depth += 1;
        let result = stacker::maybe_grow(RED_ZONE, STACK_SEGMENT, || rule(self));
        self.depth -= 1;
        result
    }

    // Lexical elements.

    /// `skipMany blank`, which never fails.
    fn blanks(&mut self) {
        let blank = |p: &mut Self| {
            p.choice(&[&|p| p.attempt(Self::comment), &|p| {
                p.label(Text::Str("space"), |p| p.satisfy(is_space))
                    .map(drop)
            }])
        };
        let _ = self.many(blank);
    }

    fn comment(&mut self) -> P {
        self.char('/')?;
        self.choice(&[
            &|p| {
                p.char('*')?;
                p.skip_till(|p| p.attempt(|p| p.string("*/")))
            },
            &|p| {
                p.char('/')?;
                p.skip_till(|p| {
                    p.choice(&[
                        &|p| {
                            p.label(Text::Str("lf new-line"), |p| p.char('\n'))
                                .map(drop)
                        },
                        &Self::eof,
                    ])
                })
            },
        ])
    }

    /// A one-character token and the blanks after it.
    fn symbol(&mut self, c: char) -> P {
        self.char(c)?;
        self.blanks();
        Ok(())
    }

    /// `keyword w`: `w` not followed by an identifier's character, and
    /// the blanks after it.
    fn keyword(&mut self, word: &'static str) -> P {
        self.attempt(|p| {
            p.string(word)?;
            p.not_followed_by(Self::identifier_tail)
        })?;
        self.blanks();
        Ok(())
    }

    /// `choice (map keyword words)`.
    fn keywords(&mut self, words: &[&'static str]) -> P {
        let start = self.pos;
        for &word in words {
            match self.keyword(word) {
                Err(Fail) if self.pos == start => {}
                result => return result,
            }
        }
        Err(Fail)
    }

    /// `identifierTail`: `alphaNum <|> char '_'`.
    fn identifier_tail(&mut self) -> P<char> {
        self.choice(&[
            &|p| p.label(Text::Str("letter or digit"), |p| p.satisfy(is_alpha_num)),
            &|p| p.char('_'),
        ])
    }

    /// An identifier: its check fails after consuming it.
    fn identifier(&mut self) -> P {
        let start = self.pos;
        self.choice(&[
            &|p| p.label(Text::Str("letter"), |p| p.satisfy(is_letter)),
            &|p| p.char('_'),
        ])?;
        self.many(|p| p.identifier_tail().map(drop))?;
        let name = &self.text[start..self.pos];
        let problem = if RESERVED_WORDS.contains(&name) {
            "is reserved"
        } else if KEYWORDS.contains(&name) {
            "is a keyword"
        } else if name.contains("__") {
            "is reserved (two consecutive underscores)"
        } else {
            self.blanks();
            return Ok(());
        };
        let message = Text::Owned(format!("{name} {problem}"));
        Err(self.fail_at(self.pos, Kind::Message, message))
    }

    /// `operator` and the tables' operators: `lexeme (try …)`.
    fn operator(&mut self, op: Op) -> P {
        self.attempt(|p| match op {
            Op::Plain(s) => p.string(s),
            Op::NoEq(s) => {
                p.string(s)?;
                p.not_followed_by(|p| p.char('='))
            }
            Op::Single(c) => {
                p.char(c)?;
                p.not_followed_by(|p| p.satisfy(|d| d == c || d == '='))
            }
        })?;
        self.blanks();
        Ok(())
    }

    fn token_with(&mut self, rule: impl FnOnce(&mut Self) -> P) -> P {
        self.attempt(rule)?;
        self.blanks();
        Ok(())
    }

    /// `many1` of a labelled digit parser.
    fn digits(&mut self, label: &'static str, f: fn(char) -> bool) -> P {
        self.many1(|p| p.label(Text::Str(label), |p| p.satisfy(f)).map(drop))
    }

    fn suffix(&mut self, letters: &str) -> P {
        self.optional(|p| p.satisfy(|c| letters.contains(c)).map(drop))
            .map(drop)
    }

    fn int_constant(&mut self) -> P {
        self.choice(&[
            // hexadecimal
            &|p| {
                p.token_with(|p| {
                    p.char('0')?;
                    p.satisfy(|c| c == 'x' || c == 'X')?;
                    p.digits("hexadecimal digit", |c| c.is_ascii_hexdigit())?;
                    p.suffix("Uu")
                })
            },
            // octal
            &|p| {
                p.token_with(|p| {
                    p.char('0')?;
                    p.digits("octal digit", |c| ('0'..='7').contains(&c))?;
                    p.suffix("Uu")
                })
            },
            // badOctal, which fails having consumed
            &|p| {
                p.token_with(|p| {
                    p.char('0')?;
                    p.digits("hexadecimal digit", |c| c.is_ascii_hexdigit())
                })?;
                let message = Text::Str("Invalid octal number");
                Err(p.fail_at(p.pos, Kind::Message, message))
            },
            // decimal
            &|p| {
                p.token_with(|p| {
                    p.digits("digit", |c| c.is_ascii_digit())?;
                    p.not_followed_by(|p| {
                        p.choice(&[&|p| p.char('.'), &|p| {
                            p.exponent()?;
                            Ok(' ')
                        }])
                    })?;
                    p.suffix("Uu")
                })
            },
        ])
    }

    fn floating_constant(&mut self) -> P {
        self.choice(&[
            // floatExponent
            &|p| {
                p.token_with(|p| {
                    p.digits("digit", |c| c.is_ascii_digit())?;
                    p.exponent()?;
                    p.suffix("Ff")
                })
            },
            // floatPoint
            &|p| {
                p.token_with(|p| {
                    p.digits("digit", |c| c.is_ascii_digit())?;
                    p.char('.')?;
                    p.many(|p| {
                        p.label(Text::Str("digit"), |p| p.satisfy(|c| c.is_ascii_digit()))
                            .map(drop)
                    })?;
                    p.optional(Self::exponent)?;
                    p.suffix("Ff")
                })
            },
            // pointFloat
            &|p| {
                p.token_with(|p| {
                    p.char('.')?;
                    p.digits("digit", |c| c.is_ascii_digit())?;
                    p.optional(Self::exponent)?;
                    p.suffix("Ff")
                })
            },
        ])
    }

    /// An exponent, a lexeme of its own.
    fn exponent(&mut self) -> P {
        self.token_with(|p| {
            p.satisfy(|c| c == 'e' || c == 'E')?;
            p.suffix("+-")?;
            p.digits("digit", |c| c.is_ascii_digit())
        })
    }

    // Expressions.

    fn primary_expression(&mut self) -> P {
        self.choice(&[
            &|p| p.attempt(Self::identifier),
            &Self::int_constant,
            &Self::floating_constant,
            &|p| p.keyword("true"),
            &|p| p.keyword("false"),
            &|p| {
                p.symbol('(')?;
                p.expression()?;
                p.symbol(')')
            },
        ])
    }

    fn postfix_expression(&mut self) -> P {
        self.choice(&[
            &|p| p.attempt(Self::function_call_generic),
            &Self::primary_expression,
        ])?;
        self.many(|p| {
            p.choice(&[
                &|p| {
                    p.symbol('[')?;
                    p.expression()?;
                    p.symbol(']')
                },
                &|p| {
                    p.token_with(|p| {
                        p.string(".")?;
                        p.function_call_generic()
                    })
                },
                &|p| {
                    p.token_with(|p| {
                        p.string(".")?;
                        p.identifier()
                    })
                },
                &|p| p.operator(Op::Plain("++")),
                &|p| p.operator(Op::Plain("--")),
            ])
        })
    }

    fn function_call_generic(&mut self) -> P {
        // functionIdentifier
        self.choice(&[&|p| p.attempt(Self::identifier), &Self::type_specifier])?;
        self.symbol('(')?;
        self.choice(&[&|p| p.keyword("void"), &|p| {
            p.sep_by(Self::assignment_expression, |p| p.symbol(','))
        }])?;
        self.symbol(')')
    }

    fn unary_expression(&mut self) -> P {
        self.many(|p| {
            p.choice(&[
                &|p| p.operator(Op::Plain("++")),
                &|p| p.operator(Op::Plain("--")),
                &|p| p.operator(Op::Plain("+")),
                &|p| p.operator(Op::Plain("-")),
                &|p| p.operator(Op::Plain("!")),
                &|p| p.operator(Op::Plain("~")),
            ])
        })?;
        self.postfix_expression()
    }

    /// `buildExpressionParser`: a level for each row of `table`, reading
    /// terms of the rows before joined by the row's operators. No row
    /// mixes associativities, so its operators are tried twice before the
    /// first (once to call them ambiguous) and once before each after,
    /// failing the same way each time.
    fn levels(&mut self, table: &[&[Op]], term: fn(&mut Self) -> P) -> P {
        let Some((ops, inner)) = table.split_last() else {
            return term(self);
        };
        // `termP`, between prefix and postfix operators the row lacks.
        let term_p = |p: &mut Self| -> P {
            p.no_operator();
            p.levels(inner, term)?;
            p.no_operator();
            Ok(())
        };
        term_p(self)?;
        if !self.label(Text::Str("operator"), |p| p.operators(ops))? {
            return Ok(());
        }
        loop {
            term_p(self)?;
            if !self.operators(ops)? {
                return Ok(());
            }
        }
    }

    /// `prefixP` or `postfixP` with no operators: `choice [] <?> ""`,
    /// whose error expects nothing but is known.
    fn no_operator(&mut self) {
        let nothing = Error::new(self.pos, Kind::Expect, Text::Str(""));
        self.error.merge(nothing);
    }

    /// The `choice` of a row's operators: whether one was there. Each is
    /// `try`, so none fails having consumed.
    fn operators(&mut self, ops: &[Op]) -> P<bool> {
        for &op in ops {
            if self.operator(op).is_ok() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn conditional_expression(&mut self) -> P {
        self.levels(CONDITIONAL, Self::unary_expression)?;
        self.optional(|p| {
            p.symbol('?')?;
            p.expression()?;
            p.symbol(':')?;
            p.assignment_expression()
        })
        .map(drop)
    }

    fn assignment_expression(&mut self) -> P {
        self.guarded(|p| p.levels(ASSIGNMENT, Self::conditional_expression))
    }

    fn expression(&mut self) -> P {
        self.guarded(|p| p.levels(&[&[Op::Plain(",")]], Self::assignment_expression))
    }

    fn constant_expression(&mut self) -> P {
        self.guarded(Self::conditional_expression)
    }

    // Declarations and types.

    fn declaration(&mut self) -> P {
        self.choice(&[
            &|p| {
                p.attempt(|p| {
                    p.fully_specified_type()?;
                    p.sep_by(Self::init_declarator, |p| p.symbol(','))?;
                    p.symbol(';')
                })
            },
            &|p| {
                p.keyword("invariant")?;
                p.sep_by(Self::init_declarator, |p| p.symbol(','))?;
                p.symbol(';')
            },
            &|p| {
                p.keyword("precision")?;
                p.precision_qualifier()?;
                p.type_specifier_no_precision()?;
                p.symbol(';')
            },
            &|p| {
                p.type_qualifier()?;
                p.choice(&[&|p| p.symbol(';'), &|p| {
                    p.identifier()?;
                    p.symbol('{')?;
                    p.many1(Self::struct_declaration)?;
                    p.symbol('}')?;
                    p.optional(|p| {
                        p.identifier()?;
                        p.optional(|p| {
                            p.symbol('[')?;
                            p.optional(Self::constant_expression)?;
                            p.symbol(']')
                        })
                        .map(drop)
                    })?;
                    p.symbol(';')
                }])
            },
        ])
    }

    /// `idecl`.
    fn init_declarator(&mut self) -> P {
        self.identifier()?;
        self.optional(|p| {
            p.symbol('[')?;
            p.optional(Self::constant_expression)?;
            p.symbol(']')
        })?;
        self.optional(|p| {
            p.symbol('=')?;
            p.assignment_expression()
        })
        .map(drop)
    }

    fn function_prototype(&mut self) -> P {
        self.fully_specified_type()?;
        self.identifier()?;
        self.symbol('(')?;
        self.sep_by(Self::parameter_declaration, |p| p.symbol(','))?;
        self.symbol(')')
    }

    fn parameter_declaration(&mut self) -> P {
        self.optional(|p| p.keyword("const"))?;
        // Without a keyword's boundary: `int` reads as `in` `t`.
        self.optional(|p| {
            p.choice(&[
                &|p| p.token_with(|p| p.string("inout")),
                &|p| p.token_with(|p| p.string("in")),
                &|p| p.token_with(|p| p.string("out")),
            ])
        })?;
        self.type_specifier()?;
        self.optional(|p| {
            p.identifier()?;
            p.optional(|p| {
                p.symbol('[')?;
                p.constant_expression()?;
                p.symbol(']')
            })
            .map(drop)
        })
        .map(drop)
    }

    fn fully_specified_type(&mut self) -> P {
        self.choice(&[&|p| p.attempt(Self::type_specifier), &|p| {
            p.type_qualifier()?;
            p.type_specifier()
        }])
    }

    fn interpolation_qualifier(&mut self) -> P {
        self.keywords(&["smooth", "flat", "noperspective"])
    }

    fn layout_qualifier(&mut self) -> P {
        self.keyword("layout")?;
        self.symbol('(')?;
        self.sep_by(
            |p| {
                p.identifier()?;
                p.optional(|p| {
                    p.symbol('=')?;
                    p.int_constant()
                })
                .map(drop)
            },
            |p| p.symbol(','),
        )?;
        self.symbol(')')
    }

    fn type_qualifier(&mut self) -> P {
        self.choice(&[
            &Self::storage_qualifier,
            &|p| {
                p.layout_qualifier()?;
                p.optional(Self::storage_qualifier).map(drop)
            },
            &|p| {
                p.interpolation_qualifier()?;
                p.optional(Self::storage_qualifier).map(drop)
            },
            &|p| {
                p.keyword("invariant")?;
                p.choice(&[
                    &|p| {
                        p.interpolation_qualifier()?;
                        p.storage_qualifier()
                    },
                    &|p| p.optional(Self::storage_qualifier).map(drop),
                ])
            },
        ])
    }

    fn storage_qualifier(&mut self) -> P {
        self.choice(&[
            &|p| p.keywords(&["const", "attribute", "varying", "in", "out"]),
            &|p| {
                p.keyword("centroid")?;
                p.keywords(&["varying", "in", "out"])
            },
            &|p| p.keyword("uniform"),
        ])
    }

    fn type_specifier(&mut self) -> P {
        self.choice(&[
            &|p| {
                p.attempt(Self::precision_qualifier)?;
                p.type_specifier_no_precision()
            },
            &Self::type_specifier_no_precision,
        ])
    }

    fn type_specifier_no_precision(&mut self) -> P {
        // typeSpecifierNonArray
        self.choice(&[
            &|p| p.keywords(TYPES),
            &Self::struct_specifier,
            &Self::identifier,
        ])?;
        self.choice(&[
            &|p| {
                p.attempt(|p| {
                    p.symbol('[')?;
                    p.symbol(']')
                })
            },
            &|p| {
                p.symbol('[')?;
                p.constant_expression()?;
                p.symbol(']')
            },
            &|_| Ok(()),
        ])
    }

    fn precision_qualifier(&mut self) -> P {
        self.keywords(&["highp", "mediump", "lowp"])
    }

    fn struct_specifier(&mut self) -> P {
        self.keyword("struct")?;
        self.optional(Self::identifier)?;
        self.symbol('{')?;
        self.many1(Self::struct_declaration)?;
        self.symbol('}')
    }

    fn struct_declaration(&mut self) -> P {
        self.guarded(|p| {
            p.optional(Self::type_qualifier)?;
            p.type_specifier()?;
            p.sep_by(
                |p| {
                    p.identifier()?;
                    p.optional(|p| {
                        p.symbol('[')?;
                        p.optional(Self::constant_expression)?;
                        p.symbol(']')
                    })
                    .map(drop)
                },
                |p| p.symbol(','),
            )?;
            p.symbol(';')
        })
    }

    // Statements.

    fn statement(&mut self) -> P {
        self.guarded(|p| p.choice(&[&Self::compound_statement, &Self::simple_statement]))
    }

    fn simple_statement(&mut self) -> P {
        self.choice(&[
            &Self::declaration,
            &Self::expression_statement,
            &Self::selection_statement,
            &Self::switch_statement,
            &Self::case_label,
            &Self::iteration_statement,
            &Self::jump_statement,
        ])
    }

    fn compound_statement(&mut self) -> P {
        self.choice(&[
            &|p| {
                p.attempt(|p| {
                    p.symbol('{')?;
                    p.symbol('}')
                })
            },
            &|p| {
                p.symbol('{')?;
                p.many1(Self::statement)?;
                p.symbol('}')
            },
        ])
    }

    fn expression_statement(&mut self) -> P {
        self.choice(&[&|p| p.symbol(';'), &|p| {
            p.expression()?;
            p.symbol(';')
        }])
    }

    fn selection_statement(&mut self) -> P {
        self.keyword("if")?;
        self.symbol('(')?;
        self.expression()?;
        self.symbol(')')?;
        self.statement()?;
        self.optional(|p| {
            p.keyword("else")?;
            p.statement()
        })
        .map(drop)
    }

    fn condition(&mut self) -> P {
        self.choice(&[&Self::expression, &|p| {
            p.fully_specified_type()?;
            p.identifier()?;
            p.symbol('=')?;
            p.assignment_expression()
        }])
    }

    fn switch_statement(&mut self) -> P {
        self.keyword("switch")?;
        self.symbol('(')?;
        self.expression()?;
        self.symbol(')')?;
        self.symbol('{')?;
        self.many(Self::statement)?;
        self.symbol('}')
    }

    fn case_label(&mut self) -> P {
        self.choice(&[
            &|p| {
                p.keyword("case")?;
                p.expression()?;
                p.symbol(':')
            },
            &|p| {
                p.keyword("default")?;
                p.symbol(':')
            },
        ])
    }

    fn iteration_statement(&mut self) -> P {
        self.choice(&[
            &|p| {
                p.keyword("while")?;
                p.symbol('(')?;
                p.condition()?;
                p.symbol(')')?;
                p.statement()
            },
            &|p| {
                p.keyword("do")?;
                p.statement()?;
                p.keyword("while")?;
                p.symbol('(')?;
                p.expression()?;
                p.symbol(')')?;
                p.symbol(';')
            },
            &|p| {
                p.keyword("for")?;
                p.symbol('(')?;
                // forInitStatement
                p.choice(&[&Self::expression_statement, &Self::declaration])?;
                p.optional(Self::condition)?;
                p.symbol(';')?;
                p.optional(Self::expression)?;
                p.symbol(')')?;
                p.statement()
            },
        ])
    }

    fn jump_statement(&mut self) -> P {
        self.choice(&[
            &|p| {
                p.keyword("continue")?;
                p.symbol(';')
            },
            &|p| {
                p.keyword("break")?;
                p.symbol(';')
            },
            &|p| {
                p.attempt(|p| {
                    p.keyword("return")?;
                    p.symbol(';')
                })
            },
            &|p| {
                p.keyword("return")?;
                p.expression()?;
                p.symbol(';')
            },
            &|p| {
                p.keyword("discard")?;
                p.symbol(';')
            },
        ])
    }

    fn external_declaration(&mut self) -> P {
        self.choice(&[
            &|p| {
                p.attempt(Self::function_prototype)?;
                p.choice(&[&|p| p.symbol(';'), &Self::compound_statement])
            },
            &Self::declaration,
        ])
    }
}

#[cfg(test)]
mod tests;
