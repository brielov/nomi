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
//! so the reported offset is where elm reports the problem.

use icu_properties::GeneralCategory;

use crate::lexer::GENERAL_CATEGORY;
use crate::parser::{DEPTH_LIMIT, RED_ZONE, STACK_SEGMENT};

/// Why a shader doesn't parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rejection {
    /// Upstream fails, at this byte offset.
    At(usize),
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
        (None, Err(Fail)) => Err(Rejection::At(p.error.offset)),
    }
}

/// A rule failed. Whether it consumed input shows in the position.
struct Fail;

type P<T = ()> = Result<T, Fail>;

/// Parsec's `ParseError`, as far as its position goes: where, and whether
/// it carries a message. Errors that don't are dropped in a merge.
#[derive(Clone, Copy, Debug)]
struct Error {
    offset: usize,
    known: bool,
}

impl Error {
    fn unknown(offset: usize) -> Error {
        Error {
            offset,
            known: false,
        }
    }

    /// Parsec's `mergeError`: the one with a message, else the later.
    fn merge(self, other: Error) -> Error {
        match (self.known, other.known) {
            (true, false) => self,
            (false, true) => other,
            _ if other.offset > self.offset => other,
            _ => self,
        }
    }
}

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

const KEYWORDS: &[&str] = &[
    "attribute",
    "const",
    "uniform",
    "varying",
    "layout",
    "centroid",
    "flat",
    "smooth",
    "noperspective",
    "break",
    "continue",
    "do",
    "for",
    "while",
    "switch",
    "case",
    "default",
    "if",
    "else",
    "in",
    "out",
    "inout",
    "float",
    "int",
    "void",
    "bool",
    "true",
    "false",
    "invariant",
    "discard",
    "return",
    "mat2",
    "mat3",
    "mat4",
    "mat2x2",
    "mat2x3",
    "mat2x4",
    "mat3x2",
    "mat3x3",
    "mat3x4",
    "mat4x2",
    "mat4x3",
    "mat4x4",
    "vec2",
    "vec3",
    "vec4",
    "ivec2",
    "ivec3",
    "ivec4",
    "bvec2",
    "bvec3",
    "bvec4",
    "uint",
    "uvec2",
    "uvec3",
    "uvec4",
    "lowp",
    "mediump",
    "highp",
    "precision",
    "sampler1D",
    "sampler2D",
    "sampler3D",
    "samplerCube",
    "sampler1DShadow",
    "sampler2DShadow",
    "samplerCubeShadow",
    "sampler1DArray",
    "sampler2DArray",
    "sampler1DArrayShadow",
    "sampler2DArrayShadow",
    "isampler1D",
    "isampler2D",
    "isampler3D",
    "isamplerCube",
    "isampler1DArray",
    "isampler2DArray",
    "usampler1D",
    "usampler2D",
    "usampler3D",
    "usamplerCube",
    "usampler1DArray",
    "usampler2DArray",
    "sampler2DRect",
    "sampler2DRectShadow",
    "isampler2DRect",
    "usampler2DRect",
    "samplerBuffer",
    "isamplerBuffer",
    "usamplerBuffer",
    "sampler2DMS",
    "isampler2DMS",
    "usampler2DMS",
    "sampler2DMSArray",
    "isampler2DMSArray",
    "usampler2DMSArray",
    "struct",
];

const RESERVED_WORDS: &[&str] = &[
    "common",
    "partition",
    "active",
    "asm",
    "class",
    "union",
    "enum",
    "typedef",
    "template",
    "this",
    "packed",
    "goto",
    "inline",
    "noinline",
    "volatile",
    "public",
    "static",
    "extern",
    "external",
    "interface",
    "long",
    "short",
    "double",
    "half",
    "fixed",
    "unsigned",
    "superp",
    "input",
    "output",
    "hvec2",
    "hvec3",
    "hvec4",
    "dvec2",
    "dvec3",
    "dvec4",
    "fvec2",
    "fvec3",
    "fvec4",
    "sampler3DRect",
    "filter",
    "image1D",
    "image2D",
    "image3D",
    "imageCube",
    "iimage1D",
    "iimage2D",
    "iimage3D",
    "iimageCube",
    "uimage1D",
    "uimage2D",
    "uimage3D",
    "uimageCube",
    "image1DArray",
    "image2DArray",
    "iimage1DArray",
    "iimage2DArray",
    "uimage1DArray",
    "uimage2DArray",
    "image1DShadow",
    "image2DShadow",
    "image1DArrayShadow",
    "image2DArrayShadow",
    "imageBuffer",
    "iimageBuffer",
    "uimageBuffer",
    "sizeof",
    "cast",
    "namespace",
    "using",
    "row_major",
];

/// `typeSpecifierNonArray`'s keywords, in upstream's order.
const TYPES: &[&str] = &[
    "void",
    "float",
    "int",
    "uint",
    "bool",
    "vec2",
    "vec3",
    "vec4",
    "bvec2",
    "bvec3",
    "bvec4",
    "ivec2",
    "ivec3",
    "ivec4",
    "uvec2",
    "uvec3",
    "uvec4",
    "mat2",
    "mat3",
    "mat4",
    "mat2x2",
    "mat2x3",
    "mat2x4",
    "mat3x2",
    "mat3x3",
    "mat3x4",
    "mat4x2",
    "mat4x3",
    "mat4x4",
    "sampler1D",
    "sampler2D",
    "sampler3D",
    "samplerCube",
    "sampler1DShadow",
    "sampler2DShadow",
    "samplerCubeShadow",
    "sampler1DArray",
    "sampler2DArray",
    "sampler1DArrayShadow",
    "sampler2DArrayShadow",
    "isampler1D",
    "isampler2D",
    "isampler3D",
    "isamplerCube",
    "isampler1DArray",
    "isampler2DArray",
    "usampler1D",
    "usampler2D",
    "usampler3D",
    "usamplerCube",
    "usampler1DArray",
    "usampler2DArray",
    "sampler2DRect",
    "sampler2DRectShadow",
    "isampler2DRect",
    "usampler2DRect",
    "samplerBuffer",
    "isamplerBuffer",
    "usamplerBuffer",
    "sampler2DMS",
    "isampler2DMS",
    "usampler2DMS",
    "sampler2DMSArray",
    "isampler2DMSArray",
    "usampler2DMSArray",
];

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

fn is_identifier_head(c: char) -> bool {
    is_letter(c) || c == '_'
}

fn is_identifier_tail(c: char) -> bool {
    is_alpha_num(c) || c == '_'
}

impl<'t> Glsl<'t> {
    // Parsec's primitives and combinators.

    fn peek(&self) -> Option<char> {
        self.text[self.pos..].chars().next()
    }

    /// Fails here, as a token parser does, merging the error.
    fn fail(&mut self) -> Fail {
        self.fail_at(self.pos)
    }

    fn fail_at(&mut self, offset: usize) -> Fail {
        self.error = self.error.merge(Error {
            offset,
            known: true,
        });
        Fail
    }

    /// Consumes `len` bytes, which starts the error afresh.
    fn advance(&mut self, len: usize) {
        self.pos += len;
        self.error = Error::unknown(self.pos);
    }

    fn satisfy(&mut self, f: impl Fn(char) -> bool) -> P<char> {
        match self.peek() {
            Some(c) if f(c) => {
                self.advance(c.len_utf8());
                Ok(c)
            }
            _ => Err(self.fail()),
        }
    }

    fn char(&mut self, c: char) -> P {
        self.satisfy(|d| d == c).map(drop)
    }

    /// `string`: consumes what matches, so a partial match fails having
    /// consumed input, with the error where it started.
    fn string(&mut self, s: &str) -> P {
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
        if matched == 0 {
            return Err(self.fail());
        }
        self.pos += matched;
        self.error = Error {
            offset: start,
            known: true,
        };
        Err(Fail)
    }

    fn eof(&mut self) -> P {
        if self.pos == self.text.len() {
            Ok(())
        } else {
            Err(self.fail())
        }
    }

    /// `try`: a failure consumes nothing.
    fn attempt<T>(&mut self, rule: impl FnOnce(&mut Self) -> P<T>) -> P<T> {
        let (pos, error) = (self.pos, self.error);
        let result = rule(self);
        if result.is_err() && self.pos != pos {
            self.pos = pos;
            self.error = error.merge(self.error);
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

    /// `notFollowedBy`, which never consumes.
    fn not_followed_by(&mut self, rule: impl FnOnce(&mut Self) -> P) -> P {
        let (start, error) = (self.pos, self.error);
        match self.attempt(rule) {
            Ok(()) => {
                self.fail();
                self.pos = start;
                self.error = error.merge(self.error);
                Err(Fail)
            }
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
                p.satisfy(is_space).map(drop)
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
                p.skip_till(|p| p.choice(&[&|p| p.char('\n'), &Self::eof]))
            },
        ])
    }

    /// A one-character token and the blanks after it.
    fn symbol(&mut self, c: char) -> P {
        self.char(c)?;
        self.blanks();
        Ok(())
    }

    /// `choice (map keyword words)`, where `keyword w` takes `w` not
    /// followed by an identifier's character, and the blanks after it.
    fn keywords(&mut self, words: &[&str]) -> P {
        let start = self.pos;
        let rest = &self.text[start..];
        let word_len = rest
            .find(|c: char| !is_identifier_tail(c))
            .unwrap_or(rest.len());
        let word = &rest[..word_len];
        for &w in words {
            if word == w {
                self.advance(w.len());
                self.blanks();
                return Ok(());
            }
            // `notFollowedBy` fails past the character that follows.
            let offset = match word.strip_prefix(w) {
                Some(more) => {
                    let next = more.chars().next().expect("the word is longer");
                    start + w.len() + next.len_utf8()
                }
                None => start,
            };
            self.fail_at(offset);
        }
        Err(Fail)
    }

    fn keyword(&mut self, word: &str) -> P {
        self.keywords(&[word])
    }

    /// An identifier: its check fails after consuming it.
    fn identifier(&mut self) -> P {
        let start = self.pos;
        self.satisfy(is_identifier_head)?;
        while self.satisfy(is_identifier_tail).is_ok() {}
        let name = &self.text[start..self.pos];
        if KEYWORDS.contains(&name) || RESERVED_WORDS.contains(&name) || name.contains("__") {
            return Err(self.fail());
        }
        self.blanks();
        Ok(())
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
                p.not_followed_by(|p| p.satisfy(|d| d == c || d == '=').map(drop))
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

    fn digits(&mut self, f: fn(char) -> bool) -> P {
        self.satisfy(f)?;
        while self.satisfy(f).is_ok() {}
        Ok(())
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
                    p.digits(|c| c.is_ascii_hexdigit())?;
                    p.suffix("Uu")
                })
            },
            // octal
            &|p| {
                p.token_with(|p| {
                    p.char('0')?;
                    p.digits(|c| ('0'..='7').contains(&c))?;
                    p.suffix("Uu")
                })
            },
            // badOctal, which fails having consumed
            &|p| {
                p.token_with(|p| {
                    p.char('0')?;
                    p.digits(|c| c.is_ascii_hexdigit())
                })?;
                Err(p.fail())
            },
            // decimal
            &|p| {
                p.token_with(|p| {
                    p.digits(|c| c.is_ascii_digit())?;
                    p.not_followed_by(|p| p.choice(&[&|p| p.char('.'), &Self::exponent]))?;
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
                    p.digits(|c| c.is_ascii_digit())?;
                    p.exponent()?;
                    p.suffix("Ff")
                })
            },
            // floatPoint
            &|p| {
                p.token_with(|p| {
                    p.digits(|c| c.is_ascii_digit())?;
                    p.char('.')?;
                    while p.satisfy(|c| c.is_ascii_digit()).is_ok() {}
                    p.optional(Self::exponent)?;
                    p.suffix("Ff")
                })
            },
            // pointFloat
            &|p| {
                p.token_with(|p| {
                    p.char('.')?;
                    p.digits(|c| c.is_ascii_digit())?;
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
            p.digits(|c| c.is_ascii_digit())
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

    /// `buildExpressionParser`: each level reads terms of the level
    /// before, joined by its operators. Every level here is of one
    /// associativity, so its ambiguity checks never fire.
    fn levels(&mut self, table: &[&[Op]], term: fn(&mut Self) -> P) -> P {
        let Some((ops, inner)) = table.split_last() else {
            return term(self);
        };
        self.levels(inner, term)?;
        // The operators fail without consuming, so trying each in turn is
        // their `choice`.
        while ops.iter().any(|&op| self.operator(op).is_ok()) {
            self.levels(inner, term)?;
        }
        Ok(())
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
