//! `Parse/Whitespace.hs` and `Parse/Comments.hs`: spaces, newlines and
//! comments. The whitespace a pattern's parentheses hold says whether it
//! held a comment, all that upstream's control flow reads of the
//! comments it collects: a pattern in parentheses with none around it is
//! the pattern itself.

use nomi_base::grow_large;

use super::engine::{P, R};

/// `spaces`: blanks, `{--}` and block comments, at least one, and
/// whether there was a comment. Tabs are not whitespace.
pub(super) fn spaces(p: &mut P<'_>) -> R<bool> {
    Ok(space(p)? | any_comment(p, space)?)
}

fn space(p: &mut P<'_>) -> R<bool> {
    p.alts(&mut [
        &mut |p| p.char(' ').map(|()| false),
        &mut |p| p.attempt(|p| p.string("{--}")).map(|()| true),
        &mut |p| multi_comment(p).map(|()| true),
    ])
}

/// `many parser`, and whether any of what it parsed held a comment.
fn any_comment<'s>(p: &mut P<'s>, mut parser: impl FnMut(&mut P<'s>) -> R<bool>) -> R<bool> {
    let mut comments = false;
    p.many(|p| {
        comments |= parser(p)?;
        Ok(())
    })?;
    Ok(comments)
}

/// `forcedWS`, and whether it held a comment.
pub(super) fn forced_ws(p: &mut P<'_>) -> R<bool> {
    // Neither alternative is under `try`, so the newlines after either
    // can follow the choice.
    let first = p.choice(&mut [&mut spaces, &mut nl_space])?;
    Ok(first | any_comment(p, nl_space)?)
}

fn nl_space(p: &mut P<'_>) -> R<bool> {
    p.attempt(|p| Ok(newlines1(p)? | p.option(spaces)?.unwrap_or(false)))
}

/// `many1 newline`, and whether there was a line comment.
fn newlines1(p: &mut P<'_>) -> R<bool> {
    Ok(newline(p)? | any_comment(p, newline)?)
}

/// `whitespace`, and whether it held a comment.
pub(super) fn whitespace(p: &mut P<'_>) -> R<bool> {
    Ok(p.option(forced_ws)?.unwrap_or(false))
}

/// `freshLine`: newlines, perhaps between spaces.
pub(super) fn fresh_line(p: &mut P<'_>) -> R<()> {
    p.alt(
        |p| {
            p.attempt(|p| {
                newlines1(p)?;
                p.many(space_nl).map(drop)
            })
        },
        |p| {
            p.attempt(|p| {
                space_nl(p)?;
                p.many(space_nl).map(drop)
            })
        },
    )
}

fn space_nl(p: &mut P<'_>) -> R<()> {
    p.attempt(|p| {
        spaces(p)?;
        newlines1(p).map(drop)
    })
}

/// `newline`: a line break, or a line comment and its break, and
/// whether it was a comment.
fn newline(p: &mut P<'_>) -> R<bool> {
    p.alt(
        |p| simple_newline(p).map(|()| false),
        |p| line_comment(p).map(|()| true),
    )
}

/// `simpleNewline`, which notes it on the newline stack.
pub(super) fn simple_newline(p: &mut P<'_>) -> R<()> {
    p.alt(|p| p.attempt(|p| p.string("\r\n")), |p| p.char('\n'))?;
    p.set_newline();
    Ok(())
}

fn line_comment(p: &mut P<'_>) -> R<()> {
    p.attempt(|p| p.string("--"))?;
    p.choice(&mut [
        &mut |p| {
            p.attempt(|p| {
                p.char('}')?;
                p.many(|p| p.char(' '))?;
                p.alt(simple_newline, P::eof)
            })
        },
        &mut |p| any_until(p, |p| p.alt(simple_newline, P::eof)),
    ])
}

/// `restOfLine`: spaces, and a line comment up to its break.
pub(super) fn rest_of_line(p: &mut P<'_>) -> R<()> {
    p.many(|p| p.char(' '))?;
    p.choice(&mut [
        &mut |p| {
            p.attempt(|p| p.string("--"))?;
            any_until(p, |p| p.alt(|p| p.look_ahead(simple_newline), P::eof))
        },
        &mut |_| Ok(()),
    ])
}

/// `docComment`.
pub(super) fn doc_comment(p: &mut P<'_>) -> R<()> {
    p.attempt(|p| p.string("{-|"))?;
    p.many(|p| p.char(' '))?;
    close_comment(p)
}

fn multi_comment(p: &mut P<'_>) -> R<()> {
    p.attempt(|p| {
        p.string("{-")?;
        p.not_followed_by(|p| p.char('|'))
    })?;
    p.option(|p| p.char('-'))?;
    p.many(|p| p.char(' '))?;
    close_comment(p)
}

/// `closeComment`: up to the `-}` closing it, past nested comments.
fn close_comment(p: &mut P<'_>) -> R<()> {
    grow_large(|| {
        any_until(p, |p| {
            p.choice(&mut [
                &mut |p| {
                    p.attempt(|p| {
                        p.many(|p| p.char(' '))?;
                        p.string("-}")
                    })
                },
                &mut |p| {
                    p.attempt(|p| p.string("{-"))?;
                    close_comment(p)?;
                    close_comment(p)
                },
            ])
        })
    })
}

/// `anyUntil end`: `end <|> (anyChar >> anyUntil end)`.
pub(super) fn any_until(p: &mut P<'_>, mut end: impl FnMut(&mut P<'_>) -> R<()>) -> R<()> {
    while !p.alt(|p| end(p).map(|()| true), |p| p.any_char().map(|_| false))? {}
    Ok(())
}

// COMMENTS

/// `withEol`: `parser`, and the rest of its line unless it spanned lines.
pub(super) fn with_eol<'s, T>(p: &mut P<'s>, parser: impl FnOnce(&mut P<'s>) -> R<T>) -> R<T> {
    let (value, multiline) = p.track_newline_seen(parser)?;
    if !multiline {
        rest_of_line(p)?;
    }
    Ok(value)
}

/// `padded`, and `commented`: `parser` between whitespace.
pub(super) fn padded<'s, T>(p: &mut P<'s>, parser: impl FnOnce(&mut P<'s>) -> R<T>) -> R<T> {
    whitespace(p)?;
    let value = parser(p)?;
    whitespace(p)?;
    Ok(value)
}
