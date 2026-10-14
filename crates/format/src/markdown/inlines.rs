//! Cheapskate's inline parser (`Cheapskate/Inlines.hs`).

use nomi_base::ghc::{is_alpha, is_alpha_num, is_space};

use super::scanner::{Scanner, is_escapable, is_whitespace};
use super::{Inline, LinkTarget, Reference, strip};

/// The inlines of `text`; upstream's `parseInlines`.
pub(super) fn parse_inlines(text: &str) -> Vec<Inline> {
    let mut s = Scanner::new(text);
    let inlines = s.many(inline).into_iter().flatten().collect();
    debug_assert!(s.at_end(), "a symbol is any character");
    inlines
}

/// Upstream's `pInline`.
fn inline(s: &mut Scanner) -> Option<Vec<Inline>> {
    nomi_base::grow_large(|| inline_unguarded(s))
}

fn inline_unguarded(s: &mut Scanner) -> Option<Vec<Inline>> {
    s.attempt(ascii_str)
        .or_else(|| s.attempt(space))
        .or_else(|| s.attempt(|s| enclosure('*', s)))
        .or_else(|| {
            s.attempt(|s| {
                if s.last().is_some_and(is_alpha_num) {
                    return None;
                }
                enclosure('_', s)
            })
        })
        .or_else(|| {
            s.attempt(code)
                .map(|raw| vec![Inline::Code(code_text(raw).to_owned())])
        })
        .or_else(|| s.attempt(link).map(|link| vec![link]))
        .or_else(|| s.attempt(image))
        .or_else(|| s.attempt(raw_html))
        .or_else(|| s.attempt(autolink))
        .or_else(|| s.attempt(entity))
        .or_else(|| sym(s))
}

fn str(text: impl Into<String>) -> Inline {
    Inline::Str(text.into())
}

/// Spaces and newlines: a line break after two spaces, a soft break
/// otherwise, or a space when no newline is among them.
fn space(s: &mut Scanner) -> Option<Vec<Inline>> {
    let spaces = s.take_while1(is_whitespace)?;
    Some(vec![if !spaces.contains('\n') {
        Inline::Space
    } else if spaces.starts_with("  ") {
        Inline::LineBreak
    } else {
        Inline::SoftBreak
    }])
}

/// ASCII letters and digits, or the URI they start as a scheme.
fn ascii_str(s: &mut Scanner) -> Option<Vec<Inline>> {
    let t = s.take_while1(|c| c.is_ascii_alphanumeric())?;
    if s.peek() == Some(':') && is_scheme(t) {
        return uri(t, s);
    }
    Some(vec![str(t)])
}

/// An escaped character, an escaped newline, or any other character.
fn sym(s: &mut Scanner) -> Option<Vec<Inline>> {
    let c = s.any_char()?;
    if c != '\\' {
        return Some(vec![str(c)]);
    }
    Some(vec![
        s.attempt(|s| s.satisfy(is_escapable))
            .map(str)
            .or_else(|| s.attempt(|s| s.char('\n')).map(|()| Inline::LineBreak))
            .unwrap_or_else(|| str('\\')),
    ])
}

/// IANA's URI schemes, and the unofficial `coap`, `doi` and `javascript`,
/// as upstream lists them.
const SCHEMES: &[&str] = &[
    "coap",
    "doi",
    "javascript",
    "aaa",
    "aaas",
    "about",
    "acap",
    "cap",
    "cid",
    "crid",
    "data",
    "dav",
    "dict",
    "dns",
    "file",
    "ftp",
    "geo",
    "go",
    "gopher",
    "h323",
    "http",
    "https",
    "iax",
    "icap",
    "im",
    "imap",
    "info",
    "ipp",
    "iris",
    "iris.beep",
    "iris.xpc",
    "iris.xpcs",
    "iris.lwz",
    "ldap",
    "mailto",
    "mid",
    "msrp",
    "msrps",
    "mtqp",
    "mupdate",
    "news",
    "nfs",
    "ni",
    "nih",
    "nntp",
    "opaquelocktoken",
    "pop",
    "pres",
    "rtsp",
    "service",
    "session",
    "shttp",
    "sieve",
    "sip",
    "sips",
    "sms",
    "snmp",
    "soap.beep",
    "soap.beeps",
    "tag",
    "tel",
    "telnet",
    "tftp",
    "thismessage",
    "tn3270",
    "tip",
    "tv",
    "urn",
    "vemmi",
    "ws",
    "wss",
    "xcon",
    "xcon-userid",
    "xmlrpc.beep",
    "xmlrpc.beeps",
    "xmpp",
    "z39.50r",
    "z39.50s",
    "adiumxtra",
    "afp",
    "afs",
    "aim",
    "apt",
    "attachment",
    "aw",
    "beshare",
    "bitcoin",
    "bolo",
    "callto",
    "chrome",
    "chrome-extension",
    "com-eventbrite-attendee",
    "content",
    "cvs",
    "dlna-playsingle",
    "dlna-playcontainer",
    "dtn",
    "dvb",
    "ed2k",
    "facetime",
    "feed",
    "finger",
    "fish",
    "gg",
    "git",
    "gizmoproject",
    "gtalk",
    "hcp",
    "icon",
    "ipn",
    "irc",
    "irc6",
    "ircs",
    "itms",
    "jar",
    "jms",
    "keyparc",
    "lastfm",
    "ldaps",
    "magnet",
    "maps",
    "market",
    "message",
    "mms",
    "ms-help",
    "msnim",
    "mumble",
    "mvn",
    "notes",
    "oid",
    "palm",
    "paparazzi",
    "platform",
    "proxy",
    "psyc",
    "query",
    "res",
    "resource",
    "rmi",
    "rsync",
    "rtmp",
    "secondlife",
    "sftp",
    "sgn",
    "skype",
    "smb",
    "soldat",
    "spotify",
    "ssh",
    "steam",
    "svn",
    "teamspeak",
    "things",
    "udp",
    "unreal",
    "ut2004",
    "ventrilo",
    "view-source",
    "webcal",
    "wtai",
    "wyciwyg",
    "xfire",
    "xri",
    "ymsgr",
];

/// A scheme as listed, or in upper case.
fn is_scheme(t: &str) -> bool {
    SCHEMES
        .iter()
        .any(|scheme| *scheme == t || scheme.bytes().map(|b| b.to_ascii_uppercase()).eq(t.bytes()))
}

/// The rest of a URI after its scheme, but not a last punctuation mark,
/// nor a closing parenthesis it didn't open; upstream's `pUri`.
fn uri(scheme: &str, s: &mut Scanner) -> Option<Vec<Inline>> {
    s.char(':')?;
    let mut depth = 0usize;
    let rest = s.take_while(|c| match c {
        '(' => {
            depth += 1;
            true
        }
        ')' if depth > 0 => {
            depth -= 1;
            true
        }
        ' ' | '\n' | ')' => false,
        '+' | '/' => true,
        c => !is_space(c),
    });
    let last = rest.chars().last()?;
    if ".;?!:,".contains(last) {
        let mut inlines = auto_link(&format!("{scheme}:{}", &rest[..rest.len() - 1]));
        inlines.push(str(last));
        Some(inlines)
    } else {
        Some(auto_link(&format!("{scheme}:{rest}")))
    }
}

/// Text in `*`s or `_`s: emphasis, strong, or both. A run of the
/// character before a space is just text; upstream's `pEnclosure`.
fn enclosure(c: char, s: &mut Scanner) -> Option<Vec<Inline>> {
    let run = s.take_while1(|x| x == c)?;
    if let Some(space) = s.attempt(space) {
        return Some(std::iter::once(str(run)).chain(space).collect());
    }
    let double: String = [c, c].iter().collect();
    Some(match run.chars().count() {
        3 => three(c, &double, s),
        2 => two(&double, s, Vec::new()),
        1 => one(c, &double, s, Vec::new()),
        _ => vec![str(run)],
    })
}

/// `wrap(inlines)`, or nothing for no inlines; upstream's `single`.
fn single(wrap: fn(Vec<Inline>) -> Inline, inlines: Vec<Inline>) -> Vec<Inline> {
    if inlines.is_empty() {
        Vec::new()
    } else {
        vec![wrap(inlines)]
    }
}

/// Inlines up to a `c`, as emphasis; upstream's `pOne`. `double` is `c`
/// twice.
fn one(c: char, double: &str, s: &mut Scanner, prefix: Vec<Inline>) -> Vec<Inline> {
    let contents = s.many(|s| {
        s.attempt(|s| {
            if s.peek() == Some(c) {
                return None;
            }
            inline(s)
        })
        .or_else(|| {
            s.string(double)?;
            if s.peek() == Some(c) {
                return None;
            }
            Some(two(double, s, Vec::new()))
        })
    });
    let mut inlines = prefix;
    inlines.extend(contents.into_iter().flatten());
    if s.attempt(|s| s.char(c)).is_some() {
        single(Inline::Emph, inlines)
    } else {
        inlines.insert(0, str(c));
        inlines
    }
}

/// Inlines up to `double`, a delimiter twice, as strong; upstream's
/// `pTwo`.
fn two(double: &str, s: &mut Scanner, prefix: Vec<Inline>) -> Vec<Inline> {
    let contents = s.many(|s| {
        if s.rest().starts_with(double) {
            return None;
        }
        inline(s)
    });
    let mut inlines = prefix;
    inlines.extend(contents.into_iter().flatten());
    if s.string(double).is_some() {
        single(Inline::Strong, inlines)
    } else {
        inlines.insert(0, str(double));
        inlines
    }
}

/// Inlines up to one or two `c`s, as emphasis or strong, and the rest as
/// the other; upstream's `pThree`.
fn three(c: char, double: &str, s: &mut Scanner) -> Vec<Inline> {
    let mut contents: Vec<Inline> = s
        .many(|s| {
            if s.peek() == Some(c) {
                return None;
            }
            inline(s)
        })
        .into_iter()
        .flatten()
        .collect();
    if s.string(double).is_some() {
        one(c, double, s, single(Inline::Strong, contents))
    } else if s.char(c).is_some() {
        two(double, s, single(Inline::Emph, contents))
    } else {
        contents.insert(0, str([c, c, c].iter().collect::<String>()));
        contents
    }
}

/// A code span as written: a run of backticks, text that neither starts
/// nor ends with one, and the same run again; upstream's `pCode'`.
fn code<'a>(s: &mut Scanner<'a>) -> Option<&'a str> {
    let start = s.rest();
    let ticks = s.take_while1(|c| c == '`')?;
    s.many_till(
        |s| {
            s.take_while1(|c| c != '`')
                .or_else(|| s.take_while1(|c| c == '`'))
        },
        |s| {
            s.string(ticks)?;
            (s.peek() != Some('`')).then_some(())
        },
    )?;
    Some(&start[..start.len() - s.rest().len()])
}

/// The text of the code span `raw`, without the spaces around it, as
/// its inline shows it.
fn code_text(raw: &str) -> &str {
    strip(raw.trim_matches('`'))
}

/// A link's label, with escapes resolved; upstream's `pLinkLabel`.
pub(super) fn link_label(s: &mut Scanner) -> Option<String> {
    nomi_base::grow_large(|| link_label_unguarded(s))
}

fn link_label_unguarded(s: &mut Scanner) -> Option<String> {
    s.char('[')?;
    let chunks = s.many_till(
        |s| {
            s.attempt(|s| {
                s.take_while1(|c| !matches!(c, '`' | '[' | ']' | '\\'))
                    .map(str::to_owned)
            })
            .or_else(|| s.attempt(escaped))
            .or_else(|| s.attempt(|s| link_label(s).map(|t| format!("[{t}]"))))
            .or_else(|| s.attempt(code).map(str::to_owned))
        },
        |s| s.char(']'),
    )?;
    Some(chunks.concat())
}

/// A URL, in `<…>` or without spaces and with balanced parentheses;
/// upstream's `pLinkUrl`.
fn link_url(s: &mut Scanner) -> Option<String> {
    if s.char('<').is_some() {
        let chars = s.many_till(
            |s| satisfy_escaped(s, |c| c != '\r' && c != '\n'),
            |s| s.char('>'),
        )?;
        return Some(chars.into_iter().collect());
    }
    Some(s.many(url_chunk).concat())
}

fn url_chunk(s: &mut Scanner) -> Option<String> {
    nomi_base::grow_large(|| url_chunk_unguarded(s))
}

fn url_chunk_unguarded(s: &mut Scanner) -> Option<String> {
    s.attempt(|s| {
        s.take_while1(|c| !matches!(c, ' ' | '\n' | '(' | ')' | '\\'))
            .map(str::to_owned)
    })
    .or_else(|| s.attempt(escaped))
    .or_else(|| {
        s.char('(')?;
        let inner = s.many_till(url_chunk, |s| s.char(')'))?;
        Some(format!("({})", inner.concat()))
    })
}

/// A title in quotes or parentheses; upstream's `pLinkTitle`.
fn link_title(s: &mut Scanner) -> Option<String> {
    nomi_base::grow_large(|| link_title_unguarded(s))
}

fn link_title_unguarded(s: &mut Scanner) -> Option<String> {
    let c = s.satisfy(|c| matches!(c, '"' | '\'' | '('))?;
    match s.peek() {
        Some(x) if !is_whitespace(x) && x != ')' => {}
        _ => return None,
    }
    let ender = if c == '(' { ')' } else { c };
    let chunks = s.many_till(
        |s| {
            s.attempt(|s| {
                s.take_while1(|x| x != ender && x != '\\')
                    .map(str::to_owned)
            })
            .or_else(|| s.attempt(escaped))
            .or_else(|| link_title(s).map(|t| format!("{c}{t}{ender}")))
        },
        |s| {
            s.char(ender)?;
            (!s.peek().is_some_and(is_alpha_num)).then_some(())
        },
    )?;
    Some(chunks.concat())
}

/// A reference definition; upstream's `pReference`.
pub(super) fn reference(text: &str) -> Option<Reference> {
    let mut s = Scanner::new(text);
    let label = link_label(&mut s)?;
    s.char(':')?;
    s.spnl();
    let url = link_url(&mut s)?;
    let title = s
        .attempt(|s| {
            s.spnl();
            link_title(s)
        })
        .unwrap_or_default();
    s.end()?;
    Some(Reference { label, url, title })
}

/// A backslash and the punctuation it escapes; upstream's `pEscaped`.
fn escaped(s: &mut Scanner) -> Option<String> {
    s.char('\\')?;
    s.satisfy(is_escapable).map(String::from)
}

/// A character meeting `p`, escaped or not; upstream's `pSatisfy`.
fn satisfy_escaped(s: &mut Scanner, p: impl Fn(char) -> bool) -> Option<char> {
    s.attempt(|s| s.satisfy(|c| c != '\\' && p(c))).or_else(|| {
        s.char('\\')?;
        s.satisfy(|c| is_escapable(c) && p(c))
    })
}

/// A label and, after it, the target in parentheses or another label;
/// upstream's `pLink`.
fn link(s: &mut Scanner) -> Option<Inline> {
    let raw = link_label(s)?;
    let text = parse_inlines(&raw);
    let (target, title) = s.attempt(inline_link).unwrap_or_else(|| {
        let reference = s
            .attempt(|s| {
                s.spnl();
                link_label(s)
            })
            .unwrap_or(raw);
        (LinkTarget::Ref(reference), String::new())
    });
    Some(Inline::Link {
        text,
        target,
        title,
    })
}

/// A link's target in parentheses, and its title; upstream's
/// `pInlineLink`.
fn inline_link(s: &mut Scanner) -> Option<(LinkTarget, String)> {
    s.char('(')?;
    s.spaces();
    let url = link_url(s)?;
    let title = s
        .attempt(|s| {
            s.spnl();
            let title = link_title(s)?;
            s.spaces();
            Some(title)
        })
        .unwrap_or_default();
    s.char(')')?;
    Some((LinkTarget::Url(url), title))
}

/// `!` and a link to a URL.
fn image(s: &mut Scanner) -> Option<Vec<Inline>> {
    s.char('!')?;
    Some(match s.attempt(link) {
        Some(Inline::Link {
            text,
            target: LinkTarget::Url(url),
            title,
        }) => vec![Inline::Image { text, url, title }],
        Some(other) => vec![str('!'), other],
        None => vec![str('!')],
    })
}

/// `&name;`, `&#digits;` or `&#xhex;`.
fn entity(s: &mut Scanner) -> Option<Vec<Inline>> {
    s.char('&')?;
    let name = s
        .attempt(|s| {
            s.take_while1(|c| c.is_ascii_alphabetic())
                .map(str::to_owned)
        })
        .or_else(|| {
            s.attempt(|s| {
                s.char('#')?;
                Some(format!("#{}", s.take_while1(|c| c.is_ascii_digit())?))
            })
        })
        .or_else(|| {
            s.char('#')?;
            let x = s.satisfy(|c| c == 'X' || c == 'x')?;
            Some(format!("#{x}{}", s.take_while1(|c| c.is_ascii_hexdigit())?))
        })?;
    s.char(';')?;
    Some(vec![Inline::Entity(format!("&{name};"))])
}

fn raw_html(s: &mut Scanner) -> Option<Vec<Inline>> {
    let html = s
        .attempt(|s| html_tag(s).map(|tag| tag.text))
        .or_else(|| html_comment(s))?;
    Some(vec![Inline::RawHtml(html)])
}

/// An HTML tag.
pub(super) struct HtmlTag {
    /// Lowercased.
    pub(super) name: String,
    /// The whole tag, as written.
    pub(super) text: String,
}

/// Upstream's `pHtmlTag`.
pub(super) fn html_tag(s: &mut Scanner) -> Option<HtmlTag> {
    s.char('<')?;
    let closing = s.attempt(|s| s.char('/')).is_some();
    let name = s.take_while1(|c| c.is_ascii_alphanumeric() || c == '?' || c == '!')?;
    let attributes = s.many(|s| {
        let spaces = s.take_while(is_space);
        let first = s.satisfy(is_alpha)?;
        let rest = s.take_while(|c| c.is_ascii_alphanumeric() || c == ':');
        s.char('=')?;
        let value = s
            .attempt(|s| quoted('"', s))
            .or_else(|| s.attempt(|s| quoted('\'', s)))
            .or_else(|| s.take_while1(is_alpha_num).map(str::to_owned))
            .unwrap_or_default();
        Some(format!("{spaces}{first}{rest}={value}"))
    });
    let last = s.take_while(|c| is_space(c) || c == '/');
    s.char('>')?;
    let slash = if closing { "/" } else { "" };
    Some(HtmlTag {
        name: name.to_lowercase(),
        text: format!("<{slash}{name}{}{last}>", attributes.concat()),
    })
}

fn quoted(c: char, s: &mut Scanner) -> Option<String> {
    s.char(c)?;
    let contents = s.take_while(|x| x != c);
    s.char(c)?;
    Some(format!("{c}{contents}{c}"))
}

fn html_comment(s: &mut Scanner) -> Option<String> {
    s.string("<!--")?;
    let chars = s.many_till(Scanner::any_char, |s| s.string("-->"))?;
    Some(format!("<!--{}-->", chars.into_iter().collect::<String>()))
}

/// `<scheme:…>` or `<name@host>`; upstream's `pAutolink`.
fn autolink(s: &mut Scanner) -> Option<Vec<Inline>> {
    s.char('<')?;
    let scheme = s.take_while1(|c| c != ':' && c != '@')?;
    let rest = s.take_while1(|c| c != '>' && c != ' ')?;
    s.char('>')?;
    let whole = format!("{scheme}{rest}");
    if rest.starts_with('@') {
        Some(vec![Inline::Link {
            text: vec![str(whole.clone())],
            target: LinkTarget::Url(format!("mailto:{whole}")),
            title: String::new(),
        }])
    } else if is_scheme(scheme) {
        Some(auto_link(&whole))
    } else {
        None
    }
}

/// A link to `url` showing it, its entities kept apart; upstream's
/// `autoLink`.
fn auto_link(url: &str) -> Vec<Inline> {
    let mut s = Scanner::new(url);
    let text = s
        .many(|s| {
            s.attempt(|s| s.take_while1(|c| c != '&').map(|t| vec![str(t)]))
                .or_else(|| s.attempt(entity))
                .or_else(|| s.string("&").map(|()| vec![str("&")]))
        })
        .into_iter()
        .flatten()
        .collect();
    vec![Inline::Link {
        text,
        target: LinkTarget::Url(url.to_owned()),
        title: String::new(),
    }]
}
