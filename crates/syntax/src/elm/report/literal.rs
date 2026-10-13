//! The reports on literals: `toCharReport`, `toStringReport`,
//! `toEscapeReport` and `toNumberReport`, and the shader cases of
//! `toExprReport`.

use nomi_base::Compat;
use nomi_base::diagnostic::doc::{self, dull_yellow, green, reflow, stack, vcat};
use nomi_base::diagnostic::{Doc, Problem};

use super::{Reporter, to_region};
use crate::elm::error::{self, Escape, Number, Position};

impl Reporter<'_, '_> {
    /// `toCharReport`.
    pub(super) fn char(&self, char: &error::Char, cur: Position) -> Problem {
        match char {
            error::Char::CharNotUtf8 => self.not_utf8(cur),
            error::Char::CharEndless => self.at(
                "MISSING SINGLE QUOTE",
                to_region(cur),
                reflow(
                    "I thought I was parsing a character, but I got to the end of the line \
                     without seeing the closing single quote:",
                ),
                reflow("Add a closing single quote here!"),
            ),
            error::Char::CharEscape(escape) => self.escape(escape, cur),
            error::Char::CharNotString(width) => self.at(
                "NEEDS DOUBLE QUOTES",
                self.wider(cur, *width),
                Doc::from("The following string uses single quotes:"),
                stack([
                    Doc::from("Please switch to double quotes instead:"),
                    dull_yellow("'this'")
                        .append(Doc::from(" => "))
                        .append(green("\"this\""))
                        .indent(4),
                    doc::simple_note(
                        "Elm uses double quotes for strings like \"hello\", whereas it uses \
                         single quotes for individual characters like 'a' and 'ø'. This \
                         distinction helps with code like (String.any (\\c -> c == 'X') \
                         \"90210\") where you are inspecting individual characters.",
                    ),
                ]),
            ),
        }
    }

    /// `toStringReport`.
    pub(super) fn string(&self, string: &error::String, cur: Position) -> Problem {
        match string {
            error::String::StringNotUtf8 => self.not_utf8(cur),
            error::String::StringEndlessSingle => self.at(
                "ENDLESS STRING",
                to_region(cur),
                reflow("I got to the end of the line without seeing the closing double quote:"),
                stack([
                    fill![
                        "Strings look like",
                        green("\"this\""),
                        "with double quotes on each end. Is the closing double quote missing in \
                         your code?",
                    ],
                    doc::simple_note(
                        "For a string that spans multiple lines, you can use the multi-line \
                         string syntax like this:",
                    ),
                    multi_line_string_example(),
                ]),
            ),
            error::String::StringEndlessMulti => self.at(
                "ENDLESS STRING",
                self.wider(cur, 3),
                reflow("I cannot find the end of this multi-line string:"),
                stack([
                    reflow("Add a \"\"\" somewhere after this to end the string."),
                    doc::simple_note("Here is a valid multi-line string for reference:"),
                    multi_line_string_example(),
                ]),
            ),
            error::String::StringEscape(escape) => self.escape(escape, cur),
        }
    }

    /// `toEscapeReport`.
    fn escape(&self, escape: &Escape, cur: Position) -> Problem {
        match *escape {
            Escape::EscapeUnknown => self.at(
                "UNKNOWN ESCAPE",
                self.wider(cur, 2),
                reflow(
                    "Backslashes always start escaped characters, but I do not recognize this \
                     one:",
                ),
                stack([
                    reflow("Valid escape characters include:"),
                    dull_yellow(
                        vcat(
                            ["\\n", "\\r", "\\t", "\\\"", "\\'", "\\\\", "\\u{003D}"]
                                .map(Doc::from),
                        )
                        .indent(4),
                    ),
                    reflow(
                        "Do you want one of those instead? Maybe you need \\\\ to escape a \
                         backslash?",
                    ),
                    doc::simple_note(
                        "The last style lets encode ANY character by its Unicode code point. \
                         That means \\u{0009} and \\t are the same. You can use that style for \
                         anything not covered by the other six escapes!",
                    ),
                ]),
            ),
            Escape::BadUnicodeFormat(width) => self.at(
                "BAD UNICODE ESCAPE",
                self.wider(cur, width),
                reflow("I ran into an invalid Unicode escape:"),
                stack([
                    reflow("Here are some examples of valid Unicode escapes:"),
                    dull_yellow(
                        vcat(["\\u{0041}", "\\u{03BB}", "\\u{6728}", "\\u{1F60A}"].map(Doc::from))
                            .indent(4),
                    ),
                    reflow(
                        "Notice that the code point is always surrounded by curly braces. Maybe \
                         you are missing the opening or closing curly brace?",
                    ),
                ]),
            ),
            Escape::BadUnicodeCode(width) => self.at(
                "BAD UNICODE ESCAPE",
                self.wider(cur, width),
                reflow("This is not a valid code point:"),
                reflow("The valid code points are between 0 and 10FFFF inclusive."),
            ),
            Escape::BadUnicodeLength {
                width,
                digits,
                code,
            } if digits < 4 => {
                let good = format!("{}{code:X}", "0".repeat((4 - digits) as usize));
                self.at(
                    "BAD UNICODE ESCAPE",
                    self.wider(cur, width),
                    reflow("Every code point needs at least four digits:"),
                    fill!["Try", green(format!("\\u{{{good}}}")), "instead?"],
                )
            }
            Escape::BadUnicodeLength { width, .. } => self.at(
                "BAD UNICODE ESCAPE",
                self.wider(cur, width),
                reflow("This code point has too many digits:"),
                fill![
                    "Valid code points are between",
                    green("\\u{0000}"),
                    "and",
                    green("\\u{10FFFF}").append(Doc::from(",")),
                    "so try trimming any leading zeros until you have between four and six digits.",
                ],
            ),
        }
    }

    /// `toNumberReport`.
    pub(super) fn number(&self, number: &Number, cur: Position) -> Problem {
        let region = to_region(cur);
        match number {
            Number::NumberEnd => self.at(
                "WEIRD NUMBER",
                region,
                reflow("I thought I was reading a number, but I ran into some weird stuff here:"),
                stack([
                    reflow("I recognize numbers in the following formats:"),
                    vcat(["42", "3.14", "6.022e23", "0x002B"].map(Doc::from)).indent(4),
                    reflow("So is there a way to write it like one of those?"),
                ]),
            ),
            Number::NumberDot(int) => self.at(
                "WEIRD NUMBER",
                region,
                reflow("Numbers cannot end with a dot like this:"),
                fill![
                    "Switching to",
                    green(&**int),
                    "or",
                    green(format!("{int}.0")),
                    "will work though!",
                ],
            ),
            Number::NumberHexDigit => self.at(
                "WEIRD HEXIDECIMAL",
                region,
                reflow("I thought I was reading a hexidecimal number until I got here:"),
                stack([
                    reflow(
                        "Valid hexidecimal digits include 0123456789abcdefABCDEF, so I can only \
                         recognize things like this:",
                    ),
                    vcat(["0x2B", "0x002B", "0x00ffb3"].map(Doc::from)).indent(4),
                ]),
            ),
            Number::NumberNoLeadingZero => self.at(
                "LEADING ZEROS",
                region,
                reflow("I do not accept numbers with leading zeros:"),
                stack([
                    reflow("Just delete the leading zeros and it should work!"),
                    doc::simple_note(
                        "Some languages let you to specify octal numbers by adding a leading \
                         zero. So in C, writing 0111 is the same as writing 73. Some people are \
                         used to that, but others probably want it to equal 111. Either path is \
                         going to surprise people from certain backgrounds, so Elm tries to \
                         avoid this whole situation.",
                    ),
                ]),
            ),
        }
    }

    /// The `ShaderEndless` case of `toExprReport`, which 0.19.1 widens over
    /// the `[glsl|`.
    pub(super) fn shader_endless(&self, cur: Position) -> Problem {
        let region = match self.compat {
            Compat::V0_19_1 => self.wider(cur, 6),
            Compat::V0_19_2 | Compat::V0_19_3 => to_region(cur),
        };
        self.at(
            "ENDLESS SHADER",
            region,
            reflow("I cannot find the end of this shader:"),
            reflow("Add a |] somewhere after this to end the shader."),
        )
    }

    /// The `ShaderProblem` case of `toExprReport`, quoting Parsec's
    /// message.
    pub(super) fn shader_problem(&self, problem: &str, cur: Position) -> Problem {
        self.at(
            "SHADER PROBLEM",
            to_region(cur),
            reflow("I ran into a problem while parsing this GLSL block."),
            stack([
                reflow(
                    "I use a 3rd party GLSL parser for now, and I did my best to extract their \
                     error message:",
                ),
                vcat(problem.lines().filter(|l| !l.is_empty()).map(Doc::from)).indent(4),
            ]),
        )
    }

    /// The `UNEXPECTED ENCODING` report of chars, strings and shaders.
    pub(super) fn not_utf8(&self, cur: Position) -> Problem {
        self.at(
            "UNEXPECTED ENCODING",
            to_region(cur),
            reflow(
                "Elm files use UTF-8 character encoding, but I ran into something outside of \
                 that format:",
            ),
            reflow("Is there a way to convert this to a valid UTF-8 character?"),
        )
    }
}

pub(super) fn multi_line_string_example() -> Doc {
    dull_yellow(
        vcat(
            [
                "\"\"\"",
                "# Multi-line Strings",
                "",
                "- start with triple double quotes",
                "- write whatever you want",
                "- no need to escape newlines or double quotes",
                "- end with triple double quotes",
                "\"\"\"",
            ]
            .map(Doc::from),
        )
        .indent(4),
    )
}
