//! Markdown printed back; upstream's `ElmFormat/Render/Markdown.hs`, with
//! `formatDocComment`'s `cleanBlock`, which drops `(..)` from the `@docs`
//! names at the top level.

use std::fmt::Write as _;

use super::{Block, Inline, LinkTarget, ListType, Reference};

/// The text of a doc comment of `blocks`, with `format_code` giving Elm
/// code blocks their formatted code; upstream's `formatMarkdown`.
pub(crate) fn render<C>(blocks: &[Block<C>], format_code: &dyn Fn(&C) -> Option<String>) -> String {
    let initial_blanks = match blocks {
        [] => false,
        [Block::Para(inlines), ..] => matches!(
            &inlines[..],
            [Inline::Str(a), Inline::Str(b), ..] if a == "@" && b == "docs"
        ),
        _ => true,
    };
    let trailing_blanks = match blocks {
        [] => false,
        [_] => initial_blanks,
        _ => true,
    };
    let cx = Context {
        format_code,
        top_level: true,
    };
    format!(
        "{}{}{}",
        if initial_blanks { "\n\n" } else { "" },
        cx.blocks(Container::Other, blocks),
        if trailing_blanks { "\n" } else { "" },
    )
}

/// What holds the blocks `Context::blocks` prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Container {
    ListItem,
    Other,
}

/// Where a block stands; upstream's `Context`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlockContext {
    Normal,
    /// Right after a list, where an indented code block would read as
    /// part of the list's last item.
    AfterIndentedList,
}

/// Whether inline text escapes what Markdown would read as markup, the
/// flag upstream's `formatMarkdownInline` takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Escape {
    /// `\`, `` ` ``, `_` and `*` escaped.
    Markup,
    /// As it is.
    Verbatim,
}

struct Context<'a, C> {
    format_code: &'a dyn Fn(&C) -> Option<String>,
    /// Whether the blocks are the doc comment's own, whose `@docs` names
    /// `cleanBlock` cleans.
    top_level: bool,
}

impl<C> Context<'_, C> {
    fn nested(&self) -> Self {
        Context {
            format_code: self.format_code,
            top_level: false,
        }
    }

    /// Upstream's `formatMarkdown'`.
    fn blocks(&self, container: Container, blocks: &[Block<C>]) -> String {
        nomi_base::grow_large(|| {
            let separator = match (container, blocks) {
                (Container::ListItem, [Block::Para(_), Block::List { .. }]) => "",
                _ => "\n",
            };
            let mut out = String::new();
            for (i, block) in blocks.iter().enumerate() {
                if i > 0 {
                    out.push_str(separator);
                }
                let context = if i > 0 && matches!(blocks[i - 1], Block::List { .. }) {
                    BlockContext::AfterIndentedList
                } else {
                    BlockContext::Normal
                };
                out.push_str(&self.block(context, block));
            }
            out
        })
    }

    /// Upstream's `formatMardownBlock`.
    fn block(&self, context: BlockContext, block: &Block<C>) -> String {
        match block {
            Block::ElmDocs(groups) => {
                let mut out = String::new();
                for (i, terms) in groups.iter().enumerate() {
                    out.push_str(if i == 0 { "@docs " } else { "\n@docs " });
                    for (j, term) in terms.iter().enumerate() {
                        if j > 0 {
                            out.push_str(", ");
                        }
                        if self.top_level {
                            out.extend(term.split("(..)"));
                        } else {
                            out.push_str(term);
                        }
                    }
                }
                out.push('\n');
                out
            }
            Block::Para(inlines) => format!("{}\n", self::inlines(Escape::Markup, inlines)),
            Block::Header(level, inlines) => {
                format!(
                    "\n{} {}\n",
                    "#".repeat(*level),
                    self::inlines(Escape::Markup, inlines)
                )
            }
            Block::Blockquote(blocks) => {
                prefix("> ", "> ", &self.nested().blocks(Container::Other, blocks))
            }
            Block::List { tight, kind, items } => {
                let mut out = String::new();
                for (i, item) in items.iter().enumerate() {
                    if i > 0 && !*tight {
                        out.push('\n');
                    }
                    let text = self.nested().blocks(Container::ListItem, item);
                    out.push_str(&match kind {
                        ListType::Bullet(_) => prefix("  - ", "    ", &text),
                        ListType::Numbered(..) => {
                            let n = i + 1;
                            let first = if n < 10 {
                                format!("{n}.  ")
                            } else {
                                format!("{n}. ")
                            };
                            prefix(&first, "    ", &text)
                        }
                    });
                }
                out
            }
            Block::Code { attr, text, code } => {
                let formatted = attr
                    .is_elm()
                    .then(|| (self.format_code)(code))
                    .flatten()
                    .unwrap_or_else(|| {
                        // Upstream fails on an empty block, which isn't
                        // Elm (`Text.last`); here it gets its newline.
                        if text.ends_with('\n') {
                            text.clone()
                        } else {
                            format!("{text}\n")
                        }
                    });
                if attr.is_elm() && context == BlockContext::Normal {
                    // Haskell's `lines`: no last line after a final newline.
                    formatted
                        .split_terminator('\n')
                        .fold(String::new(), |mut out, line| {
                            out.extend(["    ", line, "\n"]);
                            out
                        })
                } else {
                    format!("```{}\n{formatted}```\n", attr.lang)
                }
            }
            Block::Html(text) => format!("{text}\n"),
            Block::HRule => "---\n".to_owned(),
            Block::References(references) => {
                references
                    .iter()
                    .fold(String::new(), |mut out, Reference { label, url, title }| {
                        let _ = writeln!(out, "[{label}]: {url}{}", quoted_title(title));
                        out
                    })
            }
        }
    }
}

/// `text`'s first line after `first`, and the rest after `rest`;
/// upstream's `prefix'`.
fn prefix(first: &str, rest: &str, text: &str) -> String {
    text.split_terminator('\n')
        .enumerate()
        .fold(String::new(), |mut out, (i, line)| {
            out.extend([if i == 0 { first } else { rest }, line, "\n"]);
            out
        })
}

fn inlines(escape: Escape, inlines: &[Inline]) -> String {
    inlines.iter().map(|i| inline(escape, i)).collect()
}

/// Upstream's `formatMarkdownInline`.
fn inline(escape: Escape, inline: &Inline) -> String {
    nomi_base::grow_large(|| match inline {
        Inline::Str(text) => match escape {
            Escape::Markup => {
                let mut out = String::with_capacity(text.len());
                for c in text.chars() {
                    if matches!(c, '\\' | '`' | '_' | '*') {
                        out.push('\\');
                    }
                    out.push(c);
                }
                out
            }
            Escape::Verbatim => text.clone(),
        },
        Inline::Space => " ".to_owned(),
        Inline::SoftBreak | Inline::LineBreak => "\n".to_owned(),
        Inline::Emph(content) => format!("_{}_", inlines(Escape::Markup, content)),
        Inline::Strong(content) => format!("**{}**", inlines(Escape::Markup, content)),
        Inline::Code(text) => match longest_backtick_run(text) {
            0 => format!("`{text}`"),
            n => {
                let delimiter = "`".repeat(n + 1);
                format!("{delimiter} {text} {delimiter}")
            }
        },
        Inline::Link {
            text,
            target: LinkTarget::Url(url),
            title,
        } => {
            if title.is_empty() && is_absolute_url(url) && inlines(Escape::Verbatim, text) == *url {
                match escape {
                    Escape::Markup => format!("<{url}>"),
                    Escape::Verbatim => url.clone(),
                }
            } else {
                format!("[{}]({url}{})", inlines(escape, text), quoted_title(title))
            }
        }
        Inline::Link {
            text,
            target: LinkTarget::Ref(reference),
            ..
        } => {
            let text = inlines(escape, text);
            if text == *reference || reference.is_empty() {
                format!("[{text}]")
            } else {
                format!("[{text}][{reference}]")
            }
        }
        Inline::Image { text, url, title } => {
            format!("![{}]({url}{})", inlines(escape, text), quoted_title(title))
        }
        Inline::Entity(text) | Inline::RawHtml(text) => text.clone(),
    })
}

fn quoted_title(title: &str) -> String {
    if title.is_empty() {
        String::new()
    } else {
        format!(" \"{title}\"")
    }
}

/// Upstream's ``longestSpanOf '`'``.
fn longest_backtick_run(text: &str) -> usize {
    text.split(|c| c != '`').map(str::len).max().unwrap_or(0)
}

/// A scheme of two or more characters, a colon, and anything: upstream's
/// `absoluteUrlRegex`, whose test for the characters after the colon
/// passes them all.
fn is_absolute_url(url: &str) -> bool {
    let Some((scheme, _)) = url.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && scheme.len() >= 2
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
}
