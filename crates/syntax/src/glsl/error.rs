//! Parsec's `ParseError` (`Text/Parsec/Error.hs`), which elm quotes in a
//! SHADER PROBLEM report through `showErrorMessages`. elm 0.19.1 links
//! Parsec 3.1.13.0, 0.19.2 3.1.17.0 and 0.19.3 3.1.18.0; they differ in
//! nothing a shader's error shows.

use std::fmt::Write as _;

/// A message's kind. Parsec sorts messages by kind alone, keeping each
/// kind's in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// `SysUnExpect`: what `satisfy` met. Empty at the end of input.
    SysUnExpect,
    /// `UnExpect`, from `unexpected`.
    UnExpect,
    /// `Expect`, from `<?>`.
    Expect,
    /// `Message`, from `fail`.
    Message,
}

/// A message's text, shown only when the error is.
#[derive(Clone, Debug)]
pub(super) enum Text {
    /// As written.
    Str(&'static str),
    /// Haskell's `show` of a string, as `string` labels itself.
    Show(&'static str),
    /// `show [c]`, as `satisfy` and `char` show a character.
    ShowString(char),
    /// `show c`, as `notFollowedBy` shows what followed.
    ShowChar(char),
    Owned(String),
}

impl Text {
    fn render(&self) -> String {
        match self {
            Text::Str(s) => (*s).to_owned(),
            Text::Show(s) => show_string(s),
            Text::ShowString(c) => show_string(c.encode_utf8(&mut [0; 4])),
            Text::ShowChar(c) => show_char(*c),
            Text::Owned(s) => s.clone(),
        }
    }
}

#[derive(Clone, Debug)]
struct Message {
    kind: Kind,
    text: Text,
}

/// What a parse keeps of its errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Detail {
    /// Where they are, and whether they have messages: all a parse needs
    /// to run, since what the parser does never depends on the messages.
    Offset,
    /// Their messages too, for a report.
    Messages,
}

/// Parsec's `ParseError`: where, and the messages gathered there. One
/// without messages is unknown.
#[derive(Clone, Debug)]
pub(super) struct Error {
    pub(super) offset: usize,
    messages: Messages,
}

/// An error's messages, as its [`Detail`] keeps them.
#[derive(Clone, Debug)]
enum Messages {
    /// Only whether there are any.
    Dropped {
        known: bool,
    },
    Kept(Vec<Message>),
}

impl Error {
    pub(super) fn unknown(offset: usize, detail: Detail) -> Error {
        let messages = match detail {
            Detail::Offset => Messages::Dropped { known: false },
            Detail::Messages => Messages::Kept(Vec::new()),
        };
        Error { offset, messages }
    }

    pub(super) fn new(offset: usize, kind: Kind, text: Text, detail: Detail) -> Error {
        let messages = match detail {
            Detail::Offset => Messages::Dropped { known: true },
            Detail::Messages => Messages::Kept(vec![Message { kind, text }]),
        };
        Error { offset, messages }
    }

    pub(super) fn is_known(&self) -> bool {
        match &self.messages {
            Messages::Dropped { known } => *known,
            Messages::Kept(messages) => !messages.is_empty(),
        }
    }

    /// `mergeError self other`: the one with messages, else the later,
    /// with both's messages when they are at the same place.
    pub(super) fn merge(&mut self, other: Error) {
        match (self.is_known(), other.is_known()) {
            (true, false) => {}
            (false, true) => *self = other,
            _ if other.offset > self.offset => *self = other,
            _ if other.offset == self.offset => {
                if let (Messages::Kept(messages), Messages::Kept(others)) =
                    (&mut self.messages, other.messages)
                {
                    messages.extend(others);
                }
            }
            _ => {}
        }
    }

    /// `setExpectErrors self [label]`, what `<?>` does: the label
    /// replaces every `Expect`.
    pub(super) fn expect(&mut self, label: Text) {
        match &mut self.messages {
            Messages::Dropped { known } => *known = true,
            Messages::Kept(messages) => {
                messages.retain(|m| m.kind != Kind::Expect);
                messages.insert(
                    0,
                    Message {
                        kind: Kind::Expect,
                        text: label,
                    },
                );
            }
        }
    }

    /// `showErrorMessages "or" "unknown parse error" "expecting"
    /// "unexpected" "end of input" (errorMessages self)`, as elm calls it;
    /// an error that keeps no messages shows as unknown.
    pub(super) fn show(&self) -> String {
        let messages = match &self.messages {
            Messages::Kept(messages) if !messages.is_empty() => messages,
            _ => return "unknown parse error".to_owned(),
        };
        let of = |kind| -> Vec<String> {
            (messages.iter())
                .filter(|m| m.kind == kind)
                .map(|m| m.text.render())
                .collect()
        };
        let (sys, un) = (of(Kind::SysUnExpect), of(Kind::UnExpect));
        let show_sys = match sys.first() {
            _ if !un.is_empty() => String::new(),
            None => String::new(),
            Some(s) if s.is_empty() => "unexpected end of input".to_owned(),
            Some(s) => format!("unexpected {s}"),
        };
        let lines = clean(vec![
            show_sys,
            show_many("unexpected", un),
            show_many("expecting", of(Kind::Expect)),
            show_many("", of(Kind::Message)),
        ]);
        lines
            .iter()
            .flat_map(|line| ["\n", line.as_str()])
            .collect()
    }
}

/// `nub . filter (not . null)`.
fn clean(texts: Vec<String>) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for text in texts {
        if !text.is_empty() && !kept.contains(&text) {
            kept.push(text);
        }
    }
    kept
}

fn show_many(prefix: &str, texts: Vec<String>) -> String {
    let texts = clean(texts);
    let Some((last, init)) = texts.split_last() else {
        return String::new();
    };
    let list = if init.is_empty() {
        last.clone()
    } else {
        format!("{} or {last}", init.join(", "))
    };
    if prefix.is_empty() {
        list
    } else {
        format!("{prefix} {list}")
    }
}

/// GHC's `show` of a `String`.
fn show_string(s: &str) -> String {
    let mut out = String::from("\"");
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            out.push_str("\\\"");
        } else {
            show_lit_char(c, chars.peek().copied(), &mut out);
        }
    }
    out.push('"');
    out
}

/// GHC's `show` of a `Char`.
fn show_char(c: char) -> String {
    if c == '\'' {
        return "'\\''".to_owned();
    }
    let mut out = String::from("'");
    show_lit_char(c, Some('\''), &mut out);
    out.push('\'');
    out
}

/// GHC's `showLitChar`, given the character that follows, which decides
/// whether an escape needs `\&` to end it.
fn show_lit_char(c: char, next: Option<char>, out: &mut String) {
    const ASCII: [&str; 32] = [
        "NUL", "SOH", "STX", "ETX", "EOT", "ENQ", "ACK", "BEL", "BS", "HT", "LF", "VT", "FF", "CR",
        "SO", "SI", "DLE", "DC1", "DC2", "DC3", "DC4", "NAK", "SYN", "ETB", "CAN", "EM", "SUB",
        "ESC", "FS", "GS", "RS", "US",
    ];
    match c {
        '\u{7f}' => out.push_str("\\DEL"),
        _ if c > '\u{7f}' => {
            let _ = write!(out, "\\{}", u32::from(c));
            if next.is_some_and(|n| n.is_ascii_digit()) {
                out.push_str("\\&");
            }
        }
        '\\' => out.push_str("\\\\"),
        _ if c >= ' ' => out.push(c),
        '\u{7}' => out.push_str("\\a"),
        '\u{8}' => out.push_str("\\b"),
        '\u{c}' => out.push_str("\\f"),
        '\n' => out.push_str("\\n"),
        '\r' => out.push_str("\\r"),
        '\t' => out.push_str("\\t"),
        '\u{b}' => out.push_str("\\v"),
        '\u{e}' => {
            out.push_str("\\SO");
            if next == Some('H') {
                out.push_str("\\&");
            }
        }
        _ => {
            out.push('\\');
            out.push_str(ASCII[c as usize]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haskell_show() {
        assert_eq!(show_string("a\"b\\"), r#""a\"b\\""#);
        assert_eq!(show_string("\u{e9}1\u{e9}x"), r#""\233\&1\233x""#);
        assert_eq!(show_string("\u{e}H\u{0}\u{7f}\t"), r#""\SO\&H\NUL\DEL\t""#);
        assert_eq!(show_char('\''), r"'\''");
        assert_eq!(show_char('"'), "'\"'");
        assert_eq!(show_char('\n'), r"'\n'");
    }

    #[test]
    fn messages_show_as_parsec_does() {
        assert_eq!(
            Error::unknown(0, Detail::Messages).show(),
            "unknown parse error"
        );
        let mut error = Error::new(
            3,
            Kind::SysUnExpect,
            Text::ShowString('v'),
            Detail::Messages,
        );
        for label in ["\"/\"", "space", "\"(\"", "space"] {
            error.merge(Error::new(
                3,
                Kind::Expect,
                Text::Str(label),
                Detail::Messages,
            ));
        }
        error.merge(Error::new(
            1,
            Kind::Message,
            Text::Str("earlier"),
            Detail::Messages,
        ));
        assert_eq!(
            error.show(),
            "\nunexpected \"v\"\nexpecting \"/\", space or \"(\""
        );
        error.merge(Error::new(
            3,
            Kind::UnExpect,
            Text::ShowChar('='),
            Detail::Messages,
        ));
        error.expect(Text::Str("operator"));
        assert_eq!(error.show(), "\nunexpected '='\nexpecting operator");
        let mut end = Error::new(5, Kind::SysUnExpect, Text::Str(""), Detail::Messages);
        end.merge(Error::new(
            5,
            Kind::Message,
            Text::Str("x is reserved"),
            Detail::Messages,
        ));
        assert_eq!(end.show(), "\nunexpected end of input\nx is reserved");
    }
}
