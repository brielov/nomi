//! `Parse/Helpers.hs` and `Parse/Common.hs`: names, operators, symbols
//! and the combinators the grammar is built from.

use nomi_base::diagnostic::code::Keyword;
use nomi_base::{ghc, grow_large};

use super::engine::{Alt, P, R};
use super::whitespace::{forced_ws, padded, rest_of_line, whitespace, with_eol};

// VARIABLES

/// `innerVarChar`, 0.19's: no apostrophes.
fn inner_var_char(p: &mut P<'_>) -> R<char> {
    p.satisfy(|c| ghc::is_alpha_num(c) || c == '_')
}

/// `makeVar`: a name, failing having consumed after a reserved word.
fn make_var<'s>(p: &mut P<'s>, first: fn(char) -> bool) -> R<&'s str> {
    let start = p.pos;
    p.satisfy(first)?;
    p.many(inner_var_char)?;
    let name = &p.src()[start..p.pos];
    if Keyword::from_name(name).is_some() {
        return p.fail();
    }
    Ok(name)
}

/// `lowVar`.
pub(super) fn low_var<'s>(p: &mut P<'s>) -> R<&'s str> {
    make_var(p, ghc::is_lower)
}

/// `capVar`.
pub(super) fn cap_var<'s>(p: &mut P<'s>) -> R<&'s str> {
    make_var(p, ghc::is_upper)
}

/// `qualifiedVar`: no `try` around a qualifier, so `Foo` fails having
/// consumed.
pub(super) fn qualified_var(p: &mut P<'_>) -> R<()> {
    p.many(|p| {
        cap_var(p)?;
        p.char('.')
    })?;
    low_var(p).map(drop)
}

fn qualified_tag(p: &mut P<'_>) -> R<()> {
    p.many(|p| {
        p.attempt(|p| {
            cap_var(p)?;
            p.char('.')
        })
    })?;
    cap_var(p).map(drop)
}

/// `var`.
pub(super) fn var(p: &mut P<'_>) -> R<()> {
    p.alt(|p| p.attempt(qualified_var), qualified_tag)
}

/// `reserved`: `word`, not followed by more of a name.
pub(super) fn reserved(p: &mut P<'_>, word: &str) -> R<()> {
    p.string(word)?;
    p.not_followed_by(inner_var_char)
}

// OPERATORS

/// `anyOp`.
pub(super) fn any_op(p: &mut P<'_>) -> R<()> {
    p.alt(
        |p| betwixt(p, '`', '`', qualified_var),
        |p| sym_op(p).map(drop),
    )
}

/// `Help.isSymbol`: GHC's symbols, and these ASCII ones.
fn is_symbol(c: char) -> bool {
    ghc::is_symbol(c) || "+-/*=.$<>:&|^?%#@~!".contains(c)
}

/// `symOp`: a run of symbols that is not one of the reserved ones.
pub(super) fn sym_op<'s>(p: &mut P<'s>) -> R<&'s str> {
    let start = p.pos;
    p.many1(|p| p.satisfy(is_symbol))?;
    let op = &p.src()[start..p.pos];
    if ["=", "..", "->", "--", "|", "\u{2192}", ":"].contains(&op) {
        return p.fail();
    }
    if op == "." {
        p.not_followed_by(|p| p.satisfy(ghc::is_lower))?;
    }
    Ok(op)
}

// SYMBOLS

pub(super) fn equals(p: &mut P<'_>) -> R<()> {
    p.char('=')
}

pub(super) fn lenient_equals(p: &mut P<'_>) -> R<()> {
    p.alt(|p| p.char('='), |p| p.char(':'))
}

/// `rightArrow`: `->`, `→` or `=>`, without `try`.
pub(super) fn right_arrow(p: &mut P<'_>) -> R<()> {
    p.alts(&mut [
        &mut |p| p.string("->"),
        &mut |p| p.char('\u{2192}'),
        &mut |p| p.string("=>"),
    ])
}

pub(super) fn cons(p: &mut P<'_>) -> R<()> {
    p.string("::")
}

pub(super) fn has_type(p: &mut P<'_>) -> R<()> {
    p.char(':')
}

pub(super) fn lenient_has_type(p: &mut P<'_>) -> R<()> {
    p.alt(|p| p.char(':'), |p| p.char('='))
}

pub(super) fn comma(p: &mut P<'_>) -> R<()> {
    p.char(',')
}

pub(super) fn vertical_bar(p: &mut P<'_>) -> R<()> {
    p.char('|')
}

// SEPARATORS

/// `spaceySepBy1`.
pub(super) fn spacey_sep_by1<'s>(
    p: &mut P<'s>,
    sep: fn(&mut P<'s>) -> R<()>,
    parser: Alt<'_, 's, ()>,
) -> R<()> {
    loop {
        with_eol(p, &mut *parser)?;
        if p.option(|p| p.attempt(|p| padded(p, sep)))?.is_none() {
            return Ok(());
        }
    }
}

/// `commaSep1`, and `commaSep1'`.
pub(super) fn comma_sep1<'s>(p: &mut P<'s>, parser: Alt<'_, 's, ()>) -> R<()> {
    spacey_sep_by1(p, comma, parser)
}

/// `keyValue`.
pub(super) fn key_value<'s>(
    p: &mut P<'s>,
    sep: fn(&mut P<'s>) -> R<()>,
    key: fn(&mut P<'s>) -> R<&'s str>,
    value: fn(&mut P<'s>) -> R<&'s str>,
) -> R<()> {
    key(p)?;
    whitespace(p)?;
    sep(p)?;
    whitespace(p)?;
    value(p).map(drop)
}

/// `separated`: `e`s between `sep`s, `Some` of the value of the one `e`
/// if there was no `sep`.
pub(super) fn separated<'s, T>(
    p: &mut P<'s>,
    sep: fn(&mut P<'s>) -> R<()>,
    e: Alt<'_, 's, T>,
) -> R<Option<T>> {
    grow_large(|| {
        p.track_newline(|p| {
            let first = e(p)?;
            let arrow = p.option(|p| {
                p.attempt(|p| {
                    rest_of_line(p)?;
                    whitespace(p)?;
                    sep(p)
                })
            })?;
            if arrow.is_none() {
                return Ok(Some(first));
            }
            whitespace(p)?;
            if separated(p, sep, e)?.is_some() {
                rest_of_line(p)?;
            }
            Ok(None)
        })
    })
}

/// `dotSep1`: the count of `p`s, and the last.
pub(super) fn dot_sep1<'s>(
    p: &mut P<'s>,
    parser: fn(&mut P<'s>) -> R<&'s str>,
) -> R<(usize, &'s str)> {
    let mut last = parser(p)?;
    let n = p.many(|p| {
        p.attempt(|p| p.char('.'))?;
        last = parser(p)?;
        Ok(())
    })?;
    Ok((n + 1, last))
}

/// Whether an argument may be a `-` right after what it follows.
#[derive(Clone, Copy)]
pub(super) enum Minus {
    /// It may: `spacePrefix`.
    Anywhere,
    /// Only after whitespace: `constrainedSpacePrefix`.
    AfterSpace,
}

/// `spacePrefix` and `constrainedSpacePrefix`: arguments, each after
/// whitespace, right of the indent.
pub(super) fn space_prefix<'s, T>(p: &mut P<'s>, minus: Minus, parser: Alt<'_, 's, T>) -> R<()> {
    let spacing = |p: &mut P<'s>| -> R<()> {
        let spaced = p.option(forced_ws)?.is_some();
        match minus {
            Minus::AfterSpace if !spaced => p.not_followed_by(|p| p.char('-'))?,
            Minus::Anywhere | Minus::AfterSpace => {}
        }
        p.indented()
    };
    p.many(|p| {
        p.track_newline(|p| {
            // The first alternative's `parser` runs outside the `choice`:
            // had it failed without consuming, the second would have too.
            let bracketed = p.choice(&mut [
                &mut |p| {
                    p.attempt(|p| {
                        spacing(p)?;
                        p.look_ahead(|p| p.satisfy(|c| matches!(c, '[' | '(' | '{')))
                    })
                    .map(|_| true)
                },
                &mut |p| {
                    p.attempt(|p| {
                        spacing(p)?;
                        parser(p).map(|_| false)
                    })
                },
            ])?;
            if bracketed {
                parser(p)?;
            }
            Ok(())
        })
    })?;
    Ok(())
}

// SURROUNDED BY

/// `betwixt`.
pub(super) fn betwixt<'s, T>(
    p: &mut P<'s>,
    a: char,
    b: char,
    c: impl FnOnce(&mut P<'s>) -> R<T>,
) -> R<T> {
    p.char(a)?;
    let value = c(p)?;
    p.char(b)?;
    Ok(value)
}

/// `surround`: `p` padded between `a` and `z`, its newlines noted.
pub(super) fn surround<'s, T>(
    p: &mut P<'s>,
    a: char,
    z: char,
    parser: impl FnOnce(&mut P<'s>) -> R<T>,
) -> R<T> {
    p.track_newline(|p| {
        p.char(a)?;
        let value = padded(p, parser)?;
        p.char(z)?;
        Ok(value)
    })
}

/// A value, and whether a comment was around it.
pub(super) struct Commented<T> {
    pub(super) commented: bool,
    pub(super) value: T,
}

/// `surround''`: `inner`s separated by commas between `left` and
/// `right`, each with whether a comment was around it.
pub(super) fn surround_commented<'s, T>(
    p: &mut P<'s>,
    left: char,
    right: char,
    inner: Alt<'_, 's, T>,
) -> R<Vec<Commented<T>>> {
    p.char(left)?;
    let mut items = Vec::new();
    let pre = whitespace(p)?;
    let first = p.option(|p| {
        let value = inner(p)?;
        Ok((value, whitespace(p)?))
    })?;
    if let Some((value, post)) = first {
        items.push(Commented {
            commented: pre | post,
            value,
        });
        while p.option(|p| p.char(','))?.is_some() {
            let pre = whitespace(p)?;
            let value = inner(p)?;
            let post = whitespace(p)?;
            items.push(Commented {
                commented: pre | post,
                value,
            });
        }
    }
    p.char(right)?;
    Ok(items)
}

/// `accessible`: `e`, then any `.field`s.
pub(super) fn accessible<'s>(p: &mut P<'s>, e: impl FnOnce(&mut P<'s>) -> R<()>) -> R<()> {
    e(p)?;
    while p.option(|p| p.attempt(dot))?.is_some() {
        low_var(p)?;
    }
    Ok(())
}

fn dot(p: &mut P<'_>) -> R<()> {
    p.char('.')?;
    p.not_followed_by(|p| p.char('.'))
}

/// `commentedKeyword`.
pub(super) fn commented_keyword<'s, T>(
    p: &mut P<'s>,
    word: &str,
    parser: impl FnOnce(&mut P<'s>) -> R<T>,
) -> R<T> {
    p.attempt(|p| {
        whitespace(p)?;
        reserved(p, word)
    })?;
    whitespace(p)?;
    parser(p)
}

/// `shader`: `[glsl|` up to `|]`.
pub(super) fn shader(p: &mut P<'_>) -> R<()> {
    p.attempt(|p| p.string("[glsl|"))?;
    while !p.choice(&mut [
        &mut |p| p.attempt(|p| p.string("|]")).map(|()| true),
        &mut |p| p.any_char().map(|_| false),
    ])? {}
    Ok(())
}

// COMMON

/// `pair`.
pub(super) fn pair<'s>(
    p: &mut P<'s>,
    a: fn(&mut P<'s>) -> R<&'s str>,
    sep: fn(&mut P<'s>) -> R<()>,
    b: fn(&mut P<'s>) -> R<()>,
) -> R<()> {
    p.track_newline(|p| {
        a(p)?;
        whitespace(p)?;
        sep(p)?;
        whitespace(p)?;
        b(p)
    })
}

/// A record or a record type in its braces (`recordTerm`, `Type.record`):
/// perhaps a record it extends, then its fields, each a name, `sep` and
/// a `value`.
pub(super) fn record<'s>(
    p: &mut P<'s>,
    sep: fn(&mut P<'s>) -> R<()>,
    value: fn(&mut P<'s>) -> R<()>,
) -> R<()> {
    betwixt(p, '{', '}', |p| {
        p.track_newline(|p| {
            p.option(|p| {
                p.attempt(|p| {
                    padded(p, low_var)?;
                    p.string("|")
                })
            })?;
            sectioned_group(p, &mut |p| pair(p, low_var, sep, value))
        })
    })
}

/// `sectionedGroup`.
pub(super) fn sectioned_group<'s>(p: &mut P<'s>, term: Alt<'_, 's, ()>) -> R<()> {
    p.choice(&mut [
        &mut |p| {
            p.attempt(|p| {
                loop {
                    whitespace(p)?;
                    with_eol(p, &mut *term)?;
                    whitespace(p)?;
                    if p.option(comma)?.is_none() {
                        return Ok(());
                    }
                }
            })
        },
        &mut |p| whitespace(p).map(drop),
    ])
}
