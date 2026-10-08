//! `Parse/Whitespace.hs` and `Parse/Comments.hs`: spaces, newlines and
//! comments. Each parser returns whether it parsed a comment, all that
//! upstream's control flow reads of the comments it collects: a pattern
//! in parentheses with none around it is the pattern itself.

use super::engine::{P, R, deep};

/// `spaces`: blanks, `{--}` and block comments, at least one. Tabs are
/// not whitespace.
pub(super) fn spaces(p: &mut P<'_>) -> R<bool> {
    let mut comments = space(p)?;
    p.many(|p| {
        comments |= space(p)?;
        Ok(())
    })?;
    Ok(comments)
}

fn space(p: &mut P<'_>) -> R<bool> {
    p.alts(&mut [
        &mut |p| p.char(' ').map(|()| false),
        &mut |p| p.attempt(|p| p.string("{--}")).map(|()| true),
        &mut |p| multi_comment(p).map(|()| true),
    ])
}

/// `forcedWS`.
pub(super) fn forced_ws(p: &mut P<'_>) -> R<bool> {
    p.choice(&mut [
        &mut |p| {
            let mut comments = spaces(p)?;
            p.many(|p| {
                comments |= nl_space(p)?;
                Ok(())
            })?;
            Ok(comments)
        },
        &mut |p| {
            let mut comments = nl_space(p)?;
            p.many(|p| {
                comments |= nl_space(p)?;
                Ok(())
            })?;
            Ok(comments)
        },
    ])
}

fn nl_space(p: &mut P<'_>) -> R<bool> {
    p.attempt(|p| {
        let mut comments = newlines1(p)?;
        comments |= p.option(spaces)?.unwrap_or(false);
        Ok(comments)
    })
}

/// `many1 newline`.
fn newlines1(p: &mut P<'_>) -> R<bool> {
    let mut comments = newline(p)?;
    p.many(|p| {
        comments |= newline(p)?;
        Ok(())
    })?;
    Ok(comments)
}

/// `whitespace'`: whether there was any, and whether it held a comment.
pub(super) fn whitespace_(p: &mut P<'_>) -> R<(bool, bool)> {
    Ok(p.option(forced_ws)?.map_or((false, false), |c| (true, c)))
}

/// `whitespace`.
pub(super) fn whitespace(p: &mut P<'_>) -> R<bool> {
    whitespace_(p).map(|(_, comments)| comments)
}

/// `freshLine`: newlines, perhaps between spaces.
pub(super) fn fresh_line(p: &mut P<'_>) -> R<bool> {
    p.alt(
        |p| {
            p.attempt(|p| {
                let mut comments = newlines1(p)?;
                p.many(|p| {
                    comments |= space_nl(p)?;
                    Ok(())
                })?;
                Ok(comments)
            })
        },
        |p| {
            p.attempt(|p| {
                let mut comments = space_nl(p)?;
                p.many(|p| {
                    comments |= space_nl(p)?;
                    Ok(())
                })?;
                Ok(comments)
            })
        },
    )
}

fn space_nl(p: &mut P<'_>) -> R<bool> {
    p.attempt(|p| Ok(spaces(p)? | newlines1(p)?))
}

/// `newline`: a line break, or a line comment and its break.
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
    deep(p, |p| {
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
    let (value, multiline) = p.track_newline(parser)?;
    if !multiline {
        rest_of_line(p)?;
    }
    Ok(value)
}

/// `padded`: `parser` between whitespace, and whether either held a
/// comment.
pub(super) fn padded<'s, T>(
    p: &mut P<'s>,
    parser: impl FnOnce(&mut P<'s>) -> R<T>,
) -> R<(T, bool)> {
    let pre = whitespace(p)?;
    let value = parser(p)?;
    let post = whitespace(p)?;
    Ok((value, pre | post))
}
