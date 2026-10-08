//! Cheapskate's block parser (`Cheapskate/Parse.hs`): each line opens,
//! continues or closes containers on a stack, and the finished tree of
//! containers is then read as blocks.

use super::inlines::{html_tag, link_label, parse_inlines, reference};
use super::scanner::{Scanner, is_whitespace};
use super::{Block, CodeAttr, ListType, NumWrapper, strip};

/// The blocks of `text`, with `code` making each code block's payload of
/// its info string and text.
pub(crate) fn parse<C>(text: &str, code: &mut impl FnMut(&CodeAttr, &str) -> C) -> Vec<Block<C>> {
    let mut stack = Stack(vec![Container::new(Kind::Document)]);
    for line in super::lines(text) {
        stack.process_line(&tab_filter(line));
    }
    while stack.0.len() > 1 {
        stack.close();
    }
    let document = stack.0.pop().expect("the document");
    process_elts(&document.children, code)
}

/// Tabs as spaces to the next multiple of four; upstream's `tabFilter`,
/// which counts from each tab rather than from the line's start.
fn tab_filter(line: &str) -> String {
    let mut segments = line.split('\t').peekable();
    let mut out = String::with_capacity(line.len());
    while let Some(segment) = segments.next() {
        out.push_str(segment);
        if segments.peek().is_some() {
            let len = segment.chars().count();
            out.extend(std::iter::repeat_n(' ', 4 - len % 4));
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Document,
    BlockQuote,
    ListItem {
        marker_column: i64,
        padding: i64,
        list_type: ListType,
    },
    FencedCode {
        start_column: i64,
        fence: String,
        info: String,
    },
    IndentedCode,
    RawHtmlBlock,
    Reference,
}

#[derive(Debug)]
struct Container {
    kind: Kind,
    children: Vec<Elt>,
}

impl Container {
    fn new(kind: Kind) -> Self {
        Container {
            kind,
            children: Vec::new(),
        }
    }

    fn last_is_text(&self) -> bool {
        matches!(self.children.last(), Some(Elt::Leaf(Leaf::TextLine(_))))
    }
}

#[derive(Debug)]
enum Elt {
    Container(Container),
    Leaf(Leaf),
}

#[derive(Debug)]
enum Leaf {
    TextLine(String),
    BlankLine(String),
    AtxHeader(usize, String),
    SetextHeader(usize, String),
    Rule,
}

/// The open containers, the document first.
struct Stack(Vec<Container>);

impl Stack {
    fn top(&mut self) -> &mut Container {
        self.0.last_mut().expect("the document")
    }

    /// Upstream's `processLine`.
    fn process_line(&mut self, text: &str) {
        let (rest, unmatched) = self.try_open_containers(text);
        let offset = (text.chars().count() - rest.chars().count()) as i64;
        let top = self.0.last().expect("the document");
        let (kind, top_last_is_text) = (top.kind.clone(), top.last_is_text());
        let last_line_is_text = unmatched == 0 && top_last_is_text;
        let line = || Leaf::TextLine(rest.to_owned());
        match kind {
            Kind::RawHtmlBlock | Kind::IndentedCode if unmatched == 0 => self.add_leaf(line()),
            Kind::FencedCode { fence, .. } => {
                if rest.starts_with(&fence) {
                    self.close();
                } else {
                    self.add_leaf(line());
                }
            }
            Kind::Reference => {
                let new = try_new_containers(last_line_is_text, offset, rest);
                self.close();
                self.add_new(new);
            }
            kind => match try_new_containers(last_line_is_text, offset, rest) {
                (ns, Leaf::TextLine(t))
                    if ns.is_empty()
                        && unmatched > 0
                        && top_last_is_text
                        && kind != Kind::IndentedCode =>
                {
                    self.add_leaf(Leaf::TextLine(t));
                }
                (ns, Leaf::SetextHeader(level, _)) if ns.is_empty() && unmatched == 0 => {
                    let children = &mut self.top().children;
                    let Some(Elt::Leaf(Leaf::TextLine(t))) = children.pop() else {
                        unreachable!("a setext underline follows text")
                    };
                    children.push(Elt::Leaf(Leaf::SetextHeader(level, t)));
                }
                new => {
                    for _ in 0..unmatched {
                        self.close();
                    }
                    self.add_new(new);
                }
            },
        }
    }

    /// What is left of `text` after the open containers' continuations,
    /// and how many of them did not continue; upstream's
    /// `tryOpenContainers`.
    fn try_open_containers<'a>(&self, text: &'a str) -> (&'a str, usize) {
        let mut s = Scanner::new(text);
        for (i, container) in self.0.iter().enumerate() {
            if s.attempt(|s| container_continue(&container.kind, s))
                .is_none()
            {
                return (s.rest(), self.0.len() - i);
            }
        }
        (s.rest(), 0)
    }

    fn add_new(&mut self, (containers, leaf): (Vec<Kind>, Leaf)) {
        let fenced = matches!(containers.last(), Some(Kind::FencedCode { .. }));
        for kind in containers {
            self.0.push(Container::new(kind));
        }
        // A fenced code block starts with no blank line.
        if !(fenced && matches!(leaf, Leaf::BlankLine(_))) {
            self.add_leaf(leaf);
        }
    }

    /// Upstream's `addLeaf`: a second blank line ends a list item.
    fn add_leaf(&mut self, leaf: Leaf) {
        let top = self.top();
        if matches!(top.kind, Kind::ListItem { .. })
            && matches!(leaf, Leaf::BlankLine(_))
            && matches!(top.children.last(), Some(Elt::Leaf(Leaf::BlankLine(_))))
        {
            self.close();
            return self.add_leaf(leaf);
        }
        top.children.push(Elt::Leaf(leaf));
    }

    /// Closes the top container into its parent; upstream's
    /// `closeContainer`. A reference that doesn't parse is dropped, and a
    /// list item's last blank line moves out after it.
    fn close(&mut self) {
        if self.0.len() < 2 {
            return;
        }
        let mut top = self.0.pop().expect("a container");
        let parent = self.top();
        match top.kind {
            Kind::Reference => {
                let text = top
                    .children
                    .iter()
                    .map(text_of)
                    .collect::<Vec<_>>()
                    .join("\n");
                if reference(strip(&text)).is_some() {
                    parent.children.push(Elt::Container(top));
                }
            }
            Kind::ListItem { .. } => {
                if let Some(Elt::Leaf(Leaf::BlankLine(_))) = top.children.last() {
                    let blank = top.children.pop().expect("a blank line");
                    let empty = top.children.is_empty();
                    parent.children.push(Elt::Container(top));
                    if !empty {
                        parent.children.push(blank);
                    }
                } else {
                    parent.children.push(Elt::Container(top));
                }
            }
            _ => parent.children.push(Elt::Container(top)),
        }
    }
}

/// What a line must start with to go on in the container; upstream's
/// `containerContinue`.
fn container_continue(kind: &Kind, s: &mut Scanner) -> Option<()> {
    match kind {
        Kind::BlockQuote => {
            s.nonindent_space();
            blockquote_start(s)
        }
        Kind::IndentedCode => (0..4).try_for_each(|_| s.char(' ')),
        Kind::FencedCode { start_column, .. } => s.spaces_to_column(*start_column),
        Kind::RawHtmlBlock => (!s.sees(Scanner::blank_line)).then_some(()),
        Kind::ListItem {
            marker_column,
            padding,
            ..
        } => s.attempt(Scanner::blank_line).or_else(|| {
            s.spaces_to_column(marker_column + 1)?;
            s.up_to(padding - 1, |c| c == ' ');
            Some(())
        }),
        Kind::Reference => (!s.sees(Scanner::blank_line)
            && !s.sees(|s| {
                s.nonindent_space();
                scan_reference(s)
            }))
        .then_some(()),
        Kind::Document => Some(()),
    }
}

/// The containers a line opens and the leaf in them; upstream's
/// `tryNewContainers`, with `offset` the characters the open containers
/// took.
fn try_new_containers(last_line_is_text: bool, offset: i64, text: &str) -> (Vec<Kind>, Leaf) {
    let mut s = Scanner::new(text);
    s.column = offset + 1;
    let mut containers = s.many(container_start);
    match s.attempt(|s| verbatim_container_start(last_line_is_text, s)) {
        Some(verbatim) => {
            containers.push(verbatim);
            (containers, text_line_or_blank(&mut s))
        }
        None => (containers, leaf(last_line_is_text, &mut s)),
    }
}

fn container_start(s: &mut Scanner) -> Option<Kind> {
    s.nonindent_space();
    s.attempt(|s| blockquote_start(s).map(|()| Kind::BlockQuote))
        .or_else(|| list_marker(s))
}

/// Containers that take only text and blank lines.
fn verbatim_container_start(last_line_is_text: bool, s: &mut Scanner) -> Option<Kind> {
    s.nonindent_space();
    let after_blank = !last_line_is_text;
    s.attempt(code_fence)
        .or_else(|| {
            s.attempt(|s| {
                (after_blank && s.char(' ').is_some() && !s.sees(Scanner::blank_line))
                    .then_some(Kind::IndentedCode)
            })
        })
        .or_else(|| (after_blank && s.sees(html_block_start)).then_some(Kind::RawHtmlBlock))
        .or_else(|| (after_blank && s.sees(scan_reference)).then_some(Kind::Reference))
}

fn text_line_or_blank(s: &mut Scanner) -> Leaf {
    let rest = s.take_rest().to_owned();
    if rest.chars().all(is_whitespace) {
        Leaf::BlankLine(rest)
    } else {
        Leaf::TextLine(rest)
    }
}

/// Upstream's `leaf`.
fn leaf(last_line_is_text: bool, s: &mut Scanner) -> Leaf {
    s.nonindent_space();
    s.attempt(atx_header)
        .or_else(|| {
            s.attempt(|s| {
                let level = if last_line_is_text {
                    setext_underline(s)
                } else {
                    None
                }?;
                Some(Leaf::SetextHeader(level, String::new()))
            })
        })
        .or_else(|| s.attempt(rule).map(|()| Leaf::Rule))
        .unwrap_or_else(|| text_line_or_blank(s))
}

fn atx_header(s: &mut Scanner) -> Option<Leaf> {
    s.char('#')?;
    let level = s.up_to(5, |c| c == '#').len() + 1;
    if s.peek().is_some_and(|c| c != ' ') {
        return None;
    }
    // Closing `#`s go, unless the last is escaped.
    let text = s.take_rest().trim_end_matches([' ', '#']);
    let text = if text.ends_with('\\') {
        format!("{text}#")
    } else {
        text.to_owned()
    };
    Some(Leaf::AtxHeader(level, strip(&text).to_owned()))
}

fn setext_underline(s: &mut Scanner) -> Option<usize> {
    let d = s.satisfy(|c| c == '-' || c == '=')?;
    s.take_while(|c| c == d);
    s.blank_line()?;
    Some(if d == '=' { 1 } else { 2 })
}

/// Three or more of `*`, `_` or `-`, with spaces between if any; upstream's
/// `scanHRuleLine`.
fn rule(s: &mut Scanner) -> Option<()> {
    let c = s.satisfy(|c| c == '*' || c == '_' || c == '-')?;
    rule_rest(c, s)
}

fn rule_rest(c: char, s: &mut Scanner) -> Option<()> {
    for _ in 0..2 {
        s.spaces();
        s.char(c)?;
    }
    s.take_while(|x| x == ' ' || x == c);
    s.end()
}

fn blockquote_start(s: &mut Scanner) -> Option<()> {
    s.char('>')?;
    s.attempt(|s| s.char(' '));
    Some(())
}

fn scan_reference(s: &mut Scanner) -> Option<()> {
    s.sees(|s| {
        link_label(s)?;
        s.char(':')
    })
    .then_some(())
}

fn code_fence(s: &mut Scanner) -> Option<Kind> {
    let start_column = s.column;
    let fence = s
        .take_while1(|c| c == '`')
        .or_else(|| s.take_while1(|c| c == '~'))?;
    if fence.len() < 3 {
        return None;
    }
    s.spaces();
    let info = s.take_while(|c| c != '`' && c != '~');
    s.end()?;
    Some(Kind::FencedCode {
        start_column,
        fence: fence.to_owned(),
        info: info.to_owned(),
    })
}

/// A block-level tag or an HTML comment's start or end; upstream's
/// `parseHtmlBlockStart`.
fn html_block_start(s: &mut Scanner) -> Option<()> {
    if s.sees(|s| html_tag(s).filter(|(name, _)| BLOCK_HTML_TAGS.contains(&name.as_str())))
        || s.rest().starts_with("<!--")
        || s.rest().starts_with("-->")
    {
        Some(())
    } else {
        None
    }
}

const BLOCK_HTML_TAGS: &[&str] = &[
    "article",
    "header",
    "aside",
    "hgroup",
    "blockquote",
    "hr",
    "body",
    "li",
    "br",
    "map",
    "button",
    "object",
    "canvas",
    "ol",
    "caption",
    "output",
    "col",
    "p",
    "colgroup",
    "pre",
    "dd",
    "progress",
    "div",
    "section",
    "dl",
    "table",
    "dt",
    "tbody",
    "embed",
    "textarea",
    "fieldset",
    "tfoot",
    "figcaption",
    "th",
    "figure",
    "thead",
    "footer",
    "tr",
    "form",
    "ul",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "video",
];

/// Upstream's `parseListMarker`.
fn list_marker(s: &mut Scanner) -> Option<Kind> {
    let marker_column = s.column;
    let list_type = s.attempt(bullet).or_else(|| s.attempt(list_number))?;
    // One space of padding before a blank line or indented code.
    let padding = if s.attempt(Scanner::blank_line).is_some()
        || s.attempt(|s| {
            s.char(' ')?;
            s.sees(|s| (0..4).try_for_each(|_| s.char(' ')))
                .then_some(())
        })
        .is_some()
    {
        1
    } else {
        s.take_while(|c| c == ' ').len() as i64
    };
    if padding == 0 {
        return None;
    }
    let width = match list_type {
        ListType::Bullet(_) => 1,
        ListType::Numbered(_, n) if n < 10 => 2,
        ListType::Numbered(_, n) if n < 100 => 3,
        ListType::Numbered(_, n) if n < 1000 => 4,
        ListType::Numbered(..) => 5,
    };
    Some(Kind::ListItem {
        marker_column,
        padding: padding + width,
        list_type,
    })
}

/// A bullet that doesn't start a rule.
fn bullet(s: &mut Scanner) -> Option<ListType> {
    let c = s.satisfy(|c| c == '+' || c == '*' || c == '-')?;
    if c != '+' && s.sees(|s| rule_rest(c, s)) {
        return None;
    }
    Some(ListType::Bullet(c))
}

/// Digits, read as Haskell's `read` reads an `Int`, wrapping, and `.` or
/// `)`.
fn list_number(s: &mut Scanner) -> Option<ListType> {
    let digits = s.take_while1(|c| c.is_ascii_digit())?;
    let number = digits.bytes().fold(0i64, |n, d| {
        n.wrapping_mul(10).wrapping_add(i64::from(d - b'0'))
    });
    let wrapper = match s.satisfy(|c| c == '.' || c == ')')? {
        '.' => NumWrapper::PeriodFollowing,
        _ => NumWrapper::ParenFollowing,
    };
    Some(ListType::Numbered(wrapper, number))
}

fn text_of(elt: &Elt) -> &str {
    match elt {
        Elt::Leaf(Leaf::TextLine(t)) => t,
        _ => "",
    }
}

/// The blocks of a container's children; upstream's `processElts`.
fn process_elts<C>(elts: &[Elt], code: &mut impl FnMut(&CodeAttr, &str) -> C) -> Vec<Block<C>> {
    let mut blocks = Vec::new();
    let mut rest = elts;
    while let Some((first, after)) = rest.split_first() {
        rest = after;
        match first {
            Elt::Leaf(Leaf::TextLine(t)) => {
                if let Some(terms) = t.strip_prefix("@docs") {
                    // Every text line after an `@docs` line names more.
                    let count = rest
                        .iter()
                        .take_while(|e| matches!(e, Elt::Leaf(Leaf::TextLine(_))))
                        .count();
                    let lines = std::iter::once(terms).chain(rest[..count].iter().map(|e| {
                        let t = text_of(e);
                        t.strip_prefix("@docs").unwrap_or(t)
                    }));
                    rest = &rest[count..];
                    let groups = lines
                        .map(|line| {
                            line.split(',')
                                .map(|term| strip(term).to_owned())
                                .filter(|term| !term.is_empty())
                                .collect::<Vec<_>>()
                        })
                        .filter(|group| !group.is_empty())
                        .collect();
                    blocks.push(Block::ElmDocs(groups));
                } else {
                    let count = rest
                        .iter()
                        .take_while(|e| {
                            matches!(e, Elt::Leaf(Leaf::TextLine(t)) if !t.starts_with("@docs"))
                        })
                        .count();
                    let text = std::iter::once(t.as_str())
                        .chain(rest[..count].iter().map(text_of))
                        .map(|line| line.trim_start_matches(crate::ghc::is_space))
                        .collect::<Vec<_>>()
                        .join("\n");
                    rest = &rest[count..];
                    let text = text.trim_end_matches(crate::ghc::is_space);
                    blocks.push(Block::Para(parse_inlines(text)));
                }
            }
            Elt::Leaf(Leaf::BlankLine(_)) => {}
            Elt::Leaf(Leaf::AtxHeader(level, t) | Leaf::SetextHeader(level, t)) => {
                blocks.push(Block::Header(*level, parse_inlines(t)));
            }
            Elt::Leaf(Leaf::Rule) => blocks.push(Block::HRule),
            Elt::Container(container) => match &container.kind {
                Kind::Document => unreachable!("the document holds no document"),
                Kind::BlockQuote => {
                    blocks.push(Block::Blockquote(process_elts(&container.children, code)));
                }
                Kind::ListItem { list_type, .. } => {
                    let taken = take_list_items(*list_type, rest);
                    let (more, after) = rest.split_at(taken);
                    rest = after;
                    let items: Vec<&Container> = std::iter::once(container)
                        .chain(more.iter().filter_map(|e| match e {
                            Elt::Container(c) => Some(c),
                            Elt::Leaf(_) => None,
                        }))
                        .collect();
                    let is_blank = |e: &Elt| matches!(e, Elt::Leaf(Leaf::BlankLine(_)));
                    let tight = !more.iter().any(is_blank)
                        && items.iter().all(|c| !c.children.iter().any(is_blank));
                    blocks.push(Block::List {
                        tight,
                        kind: *list_type,
                        items: items
                            .iter()
                            .map(|c| process_elts(&c.children, code))
                            .collect(),
                    });
                }
                Kind::FencedCode { info, .. } => {
                    let (lang, info) = info.split_at(info.find(' ').unwrap_or(info.len()));
                    let attr = CodeAttr {
                        lang: lang.to_owned(),
                        info: strip(info).to_owned(),
                    };
                    let text = joined(&container.children);
                    blocks.push(code_block(attr, text, code));
                }
                Kind::IndentedCode => {
                    let count = rest
                        .iter()
                        .take_while(|e| match e {
                            Elt::Leaf(Leaf::BlankLine(_)) => true,
                            Elt::Container(c) => c.kind == Kind::IndentedCode,
                            Elt::Leaf(_) => false,
                        })
                        .count();
                    let mut lines: Vec<&str> = std::iter::once(first)
                        .chain(&rest[..count])
                        .flat_map(|e| match e {
                            // The blank line lost up to three spaces, and
                            // code four.
                            Elt::Leaf(Leaf::BlankLine(t)) => {
                                vec![t.char_indices().nth(1).map_or("", |(i, _)| &t[i..])]
                            }
                            Elt::Container(c) => c.children.iter().map(text_of).collect(),
                            Elt::Leaf(_) => Vec::new(),
                        })
                        .collect();
                    rest = &rest[count..];
                    while lines.last().is_some_and(|l| l.chars().all(|c| c == ' ')) {
                        lines.pop();
                    }
                    let attr = CodeAttr {
                        lang: String::new(),
                        info: String::new(),
                    };
                    blocks.push(code_block(attr, lines.join("\n"), code));
                }
                Kind::RawHtmlBlock => blocks.push(Block::Html(joined(&container.children))),
                Kind::Reference => {
                    // Each line of each reference in a row, parsed alone.
                    let count = rest
                        .iter()
                        .take_while(|e| matches!(e, Elt::Container(c) if c.kind == Kind::Reference))
                        .count();
                    let references = std::iter::once(first)
                        .chain(&rest[..count])
                        .flat_map(|e| match e {
                            Elt::Container(c) => c.children.iter().map(text_of).collect(),
                            Elt::Leaf(_) => Vec::new(),
                        })
                        .map(|line| {
                            reference(strip(line)).unwrap_or_else(|| {
                                ("??".to_owned(), "??".to_owned(), "??".to_owned())
                            })
                        })
                        .collect();
                    rest = &rest[count..];
                    blocks.push(Block::References(references));
                }
            },
        }
    }
    blocks
}

fn joined(elts: &[Elt]) -> String {
    elts.iter().map(text_of).collect::<Vec<_>>().join("\n")
}

fn code_block<C>(
    attr: CodeAttr,
    text: String,
    code: &mut impl FnMut(&CodeAttr, &str) -> C,
) -> Block<C> {
    let code = code(&attr, &text);
    Block::Code { attr, text, code }
}

/// How many of `elts` belong to a list of `list_type`: items of its type,
/// each after at most one blank line.
fn take_list_items(list_type: ListType, elts: &[Elt]) -> usize {
    let matches = |e: &Elt| match e {
        Elt::Container(Container {
            kind: Kind::ListItem { list_type: t, .. },
            ..
        }) => match (t, list_type) {
            (ListType::Bullet(a), ListType::Bullet(b)) => *a == b,
            (ListType::Numbered(a, _), ListType::Numbered(b, _)) => *a == b,
            _ => false,
        },
        _ => false,
    };
    let mut taken = 0;
    loop {
        match &elts[taken..] {
            [item, ..] if matches(item) => taken += 1,
            [Elt::Leaf(Leaf::BlankLine(_)), item, ..] if matches(item) => taken += 2,
            _ => return taken,
        }
    }
}

#[cfg(test)]
mod tests;
