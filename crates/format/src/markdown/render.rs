//! Markdown printed back; upstream's `ElmFormat/Render/Markdown.hs`, with
//! `formatDocComment`'s `cleanBlock`, which drops `(..)` from the `@docs`
//! names at the top level.

use super::{Block, Inline, LinkTarget, ListType, lines};

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
    cx.blocks(false, initial_blanks, trailing_blanks, blocks)
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
    fn blocks(
        &self,
        is_list_item: bool,
        initial_blanks: bool,
        trailing_blanks: bool,
        blocks: &[Block<C>],
    ) -> String {
        let separator = match (is_list_item, blocks) {
            (true, [Block::Para(_), Block::List { .. }]) => "",
            _ => "\n",
        };
        let mut out = String::new();
        if initial_blanks {
            out.push_str("\n\n");
        }
        for (i, block) in blocks.iter().enumerate() {
            if i > 0 {
                out.push_str(separator);
            }
            let after_list = i > 0 && matches!(blocks[i - 1], Block::List { .. });
            out.push_str(&self.block(after_list, block));
        }
        if trailing_blanks {
            out.push('\n');
        }
        out
    }

    /// Upstream's `formatMardownBlock`, where `after_list` is the
    /// `AfterIndentedList` context.
    fn block(&self, after_list: bool, block: &Block<C>) -> String {
        match block {
            Block::ElmDocs(groups) => {
                let groups: Vec<String> = groups
                    .iter()
                    .map(|terms| {
                        let terms: Vec<String> = terms
                            .iter()
                            .map(|t| {
                                if self.top_level {
                                    t.replace("(..)", "")
                                } else {
                                    t.clone()
                                }
                            })
                            .collect();
                        format!("@docs {}", terms.join(", "))
                    })
                    .collect();
                format!("{}\n", groups.join("\n"))
            }
            Block::Para(inlines) => format!("{}\n", self::inlines(true, inlines)),
            Block::Header(level, inlines) => {
                format!(
                    "\n{} {}\n",
                    "#".repeat(*level),
                    self::inlines(true, inlines)
                )
            }
            Block::Blockquote(blocks) => prefix(
                "> ",
                "> ",
                &self.nested().blocks(false, false, false, blocks),
            ),
            Block::List { tight, kind, items } => {
                let items = items.iter().enumerate().map(|(i, item)| {
                    let text = self.nested().blocks(true, false, false, item);
                    match kind {
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
                    }
                });
                items
                    .collect::<Vec<_>>()
                    .join(if *tight { "" } else { "\n" })
            }
            Block::Code { attr, text, code } => {
                let is_elm = attr.lang == "elm" || attr.lang.is_empty();
                let formatted = is_elm
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
                if is_elm && !after_list {
                    lines(&formatted)
                        .iter()
                        .map(|line| format!("    {line}\n"))
                        .collect()
                } else {
                    format!("```{}\n{formatted}```\n", attr.lang)
                }
            }
            Block::Html(text) => format!("{text}\n"),
            Block::HRule => "---\n".to_owned(),
            Block::References(references) => references
                .iter()
                .map(|(label, url, title)| {
                    let title = if title.is_empty() {
                        String::new()
                    } else {
                        format!(" \"{title}\"")
                    };
                    format!("[{label}]: {url}{title}\n")
                })
                .collect(),
        }
    }
}

/// `text`'s first line after `first`, and the rest after `rest`;
/// upstream's `prefix'`.
fn prefix(first: &str, rest: &str, text: &str) -> String {
    lines(text)
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{}{line}\n", if i == 0 { first } else { rest }))
        .collect()
}

fn inlines(fix: bool, inlines: &[Inline]) -> String {
    inlines.iter().map(|i| inline(fix, i)).collect()
}

/// Upstream's `formatMarkdownInline`, where `fix` escapes what Markdown
/// would read as markup.
fn inline(fix: bool, inline: &Inline) -> String {
    match inline {
        Inline::Str(text) if fix => {
            let mut out = String::with_capacity(text.len());
            for c in text.chars() {
                if matches!(c, '\\' | '`' | '_' | '*') {
                    out.push('\\');
                }
                out.push(c);
            }
            out
        }
        Inline::Str(text) => text.clone(),
        Inline::Space => " ".to_owned(),
        Inline::SoftBreak | Inline::LineBreak => "\n".to_owned(),
        Inline::Emph(content) => format!("_{}_", inlines(true, content)),
        Inline::Strong(content) => format!("**{}**", inlines(true, content)),
        Inline::Code(text) => match longest_backtick_run(text) {
            0 => format!("`{text}`"),
            n => {
                let delimiter = "`".repeat(n + 1);
                format!("{delimiter} {text} {delimiter}")
            }
        },
        Inline::Link(content, LinkTarget::Url(url), title) => {
            if title.is_empty() && is_absolute_url(url) && inlines(false, content) == *url {
                if fix { format!("<{url}>") } else { url.clone() }
            } else {
                format!("[{}]({url}{})", inlines(fix, content), quoted_title(title))
            }
        }
        Inline::Link(content, LinkTarget::Ref(reference), _) => {
            let text = inlines(fix, content);
            if text == *reference || reference.is_empty() {
                format!("[{text}]")
            } else {
                format!("[{text}][{reference}]")
            }
        }
        Inline::Image(content, url, title) => {
            format!("![{}]({url}{})", inlines(fix, content), quoted_title(title))
        }
        Inline::Entity(text) | Inline::RawHtml(text) => text.clone(),
    }
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
