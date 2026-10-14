//! Upstream's command-line parsing (`Terminal/Chomp.hs`) and its errors
//! (`Terminal/Error.hs`), as far as `elm make` needs them: flags are
//! found first, each in the order the command declares them, then any
//! flag left is unknown, then the arguments are parsed.

use nomi_base::diagnostic::doc::{Color, Doc, fill_sep, green, red, reflow, text, vcat, words};

/// `Parser`: what a flag's value or an argument is, and how to read it.
pub(crate) struct Parser<T> {
    pub(crate) singular: &'static str,
    pub(crate) parse: fn(&str) -> Option<T>,
    pub(crate) examples: &'static [&'static str],
}

/// A flag a command declares: on/off, or with a value.
enum Flag {
    OnOff(&'static str),
    Value {
        name: &'static str,
        singular: &'static str,
    },
}

impl Flag {
    fn name(&self) -> &'static str {
        match self {
            Flag::OnOff(name) | Flag::Value { name, .. } => name,
        }
    }
}

/// `Chunk`s not yet taken by a flag.
pub(crate) struct Chunks<'a> {
    chunks: Vec<&'a str>,
    /// The flags looked for so far, in order.
    declared: Vec<Flag>,
}

/// `Value`: what follows a flag.
enum Found<'a> {
    /// `--flag=value`.
    Definitely(&'a str),
    /// `--flag value`, where the value might be an argument.
    Possibly(&'a str),
    /// `--flag` with nothing after it.
    Nope,
}

impl<'a> Chunks<'a> {
    pub(crate) fn new(args: &'a [&'a str]) -> Chunks<'a> {
        Chunks {
            chunks: args.to_vec(),
            declared: Vec::new(),
        }
    }

    /// `findFlag`: takes out the first `--name` or `--name=…`, and what
    /// follows it.
    fn find(&mut self, name: &str) -> Option<(usize, Found<'a>)> {
        let lone = format!("--{name}");
        let prefix = format!("--{name}=");
        let at = (self.chunks.iter()).position(|c| c.starts_with(&prefix) || **c == lone)?;
        let chunk = self.chunks.remove(at);
        if let Some(value) = chunk.strip_prefix(&prefix) {
            return Some((at, Found::Definitely(value)));
        }
        match self.chunks.get(at) {
            Some(next) if !next.starts_with('-') => {
                Some((at, Found::Possibly(self.chunks.remove(at))))
            }
            _ => Some((at, Found::Nope)),
        }
    }

    /// `chompOnOffFlag`.
    pub(crate) fn on_off(&mut self, name: &'static str) -> Result<bool, Error> {
        self.declared.push(Flag::OnOff(name));
        match self.find(name) {
            None => Ok(false),
            Some((_, Found::Nope)) => Ok(true),
            Some((at, Found::Possibly(chunk))) => {
                // Not the flag's: back where it was, an argument.
                self.chunks.insert(at, chunk);
                Ok(true)
            }
            Some((_, Found::Definitely(value))) => Err(Error::FlagWithValue {
                flag: name,
                value: value.to_owned(),
            }),
        }
    }

    /// `chompNormalFlag`.
    pub(crate) fn value<T>(
        &mut self,
        name: &'static str,
        parser: &Parser<T>,
    ) -> Result<Option<T>, Error> {
        self.declared.push(Flag::Value {
            name,
            singular: parser.singular,
        });
        let value = match self.find(name) {
            None => return Ok(None),
            Some((_, Found::Nope)) => {
                return Err(Error::FlagWithNoValue(name, parser.expectation()));
            }
            Some((_, Found::Definitely(value) | Found::Possibly(value))) => value,
        };
        match (parser.parse)(value) {
            Some(value) => Ok(Some(value)),
            None => Err(Error::FlagWithBadValue {
                flag: name,
                value: value.to_owned(),
                expected: parser.expectation(),
            }),
        }
    }

    /// `checkForUnknownFlags`, then `zeroOrMore` arguments.
    pub(crate) fn zero_or_more<T>(self, parser: &Parser<T>) -> Result<Vec<T>, Error> {
        if let Some(unknown) = self.chunks.iter().find(|c| c.starts_with('-')) {
            return Err(Error::FlagUnknown(
                unknown.to_string(),
                nearby(unknown, &self.declared),
            ));
        }
        (self.chunks.iter())
            .map(|chunk| {
                (parser.parse)(chunk)
                    .ok_or_else(|| Error::ArgBad(chunk.to_string(), parser.expectation()))
            })
            .collect()
    }
}

impl<T> Parser<T> {
    fn expectation(&self) -> Expectation {
        Expectation {
            singular: self.singular,
            examples: self.examples,
        }
    }
}

/// `Expectation`: what a value should look like.
#[derive(Debug)]
pub(crate) struct Expectation {
    singular: &'static str,
    examples: &'static [&'static str],
}

/// The errors `elm make`'s parsing can find (`Terminal.Error`).
#[derive(Debug)]
pub(crate) enum Error {
    FlagWithValue {
        flag: &'static str,
        value: String,
    },
    FlagWithNoValue(&'static str, Expectation),
    FlagWithBadValue {
        flag: &'static str,
        value: String,
        expected: Expectation,
    },
    /// The flag, and the declared ones nearest it.
    FlagUnknown(String, Vec<String>),
    ArgBad(String, Expectation),
}

/// `getNearbyFlags`: the declared flags nearest `unknown`'s name, closest
/// first, or all of them if none is near.
fn nearby(unknown: &str, declared: &[Flag]) -> Vec<String> {
    let name: Vec<char> = (unknown.trim_start_matches('-').split('=').next())
        .unwrap_or_default()
        .chars()
        .collect();
    let mut flags: Vec<(usize, String)> = (declared.iter())
        .map(|flag| {
            let distance =
                nomi_base::suggest::distance(&name, &flag.name().chars().collect::<Vec<_>>());
            let shown = match flag {
                Flag::OnOff(name) => format!("--{name}"),
                Flag::Value { name, singular } => format!("--{name}={}", token(singular)),
            };
            (distance, shown)
        })
        .collect();
    if flags.iter().any(|(d, _)| *d < 3) {
        flags.retain(|(d, _)| *d < 3);
    }
    flags.sort_by_key(|(d, _)| *d);
    flags.into_iter().map(|(_, shown)| shown).collect()
}

/// `toToken`.
fn token(singular: &str) -> String {
    format!("<{}>", singular.replace(' ', "-"))
}

/// `singular`'s token, in yellow.
fn yellow(singular: &str) -> Doc {
    text(token(singular)).color(Color::Yellow)
}

/// The words of `before`, then `doc`, then the words of `after`, filled.
fn sentence(before: &str, doc: Doc, after: &str) -> Doc {
    fill_sep(words(before).chain([doc]).chain(words(after)))
}

/// `lines`, in green and indented as one block.
fn green_block(lines: impl IntoIterator<Item = Doc>) -> Doc {
    vcat(lines).color(Color::Green).indent(4)
}

/// `summary`, the `original` argument in red, then `explanation`.
fn help(summary: &str, original: String, explanation: impl IntoIterator<Item = Doc>) -> Vec<Doc> {
    [reflow(summary), red(original).indent(4)]
        .into_iter()
        .chain(explanation)
        .collect()
}

/// A flag's examples, each in green, or its token where it has none.
fn examples(name: &str, expected: &Expectation) -> Doc {
    let lines: Vec<String> = if expected.examples.is_empty() {
        vec![format!("--{name}={}", token(expected.singular))]
    } else {
        (expected.examples.iter().take(4))
            .map(|x| format!("--{name}={x}"))
            .collect()
    };
    vcat(lines.into_iter().map(green)).indent(4)
}

impl Error {
    /// `exitWithError`'s documents, each to be followed by a blank line.
    pub(crate) fn to_docs(&self) -> Vec<Doc> {
        match self {
            Error::FlagWithValue { flag, value } => help(
                "This on/off flag was given a value:",
                format!("--{flag}={value}"),
                [
                    vcat([
                        text(
                            "An on/off flag either exists or not. It cannot have an equals sign and value.",
                        ),
                        text("Maybe you want this instead?"),
                    ]),
                    green(format!("--{flag}")).indent(4),
                ],
            ),
            Error::FlagWithNoValue(flag, expected) => help(
                "This flag needs more information:",
                format!("--{flag}"),
                [
                    sentence("It needs a", yellow(expected.singular), "like this:"),
                    examples(flag, expected),
                ],
            ),
            Error::FlagWithBadValue {
                flag,
                value,
                expected,
            } => help(
                "This flag was given a bad value:",
                format!("--{flag}={value}"),
                [
                    sentence(
                        "I need a valid",
                        yellow(expected.singular),
                        "value. For example:",
                    ),
                    examples(flag, expected),
                ],
            ),
            Error::FlagUnknown(unknown, nearby) => {
                let explanation = match nearby.as_slice() {
                    [] => vec![],
                    [one] => vec![sentence("Maybe you want", green(one.as_str()), "instead?")],
                    many => vec![
                        reflow("Maybe you want one of these instead?"),
                        green_block(many.iter().map(|f| text(f.as_str()))),
                    ],
                };
                help(
                    "I do not recognize this flag:",
                    unknown.clone(),
                    explanation,
                )
            }
            Error::ArgBad(arg, expected) => {
                let after = if expected.examples.len() == 1 {
                    "value, like this:"
                } else {
                    "value, like one of these:"
                };
                vec![
                    text("I am having trouble with this argument:"),
                    red(arg.clone()).indent(4),
                    sentence("It is supposed to be a", yellow(expected.singular), after),
                    green_block(expected.examples.iter().map(|x| text(*x))),
                ]
            }
        }
    }
}
