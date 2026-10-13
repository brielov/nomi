//! nomi invoked as `elm-format`: elm-format 0.8.8's arguments, parsed as
//! its optparse-applicative parser parses them, with the same help and
//! errors. What the options then do is `nomi_engine::elm_format`.

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use nomi_engine::Vfs;
use nomi_engine::console::StdConsole;
use nomi_engine::elm_format::{self, ElmVersion, Exit, Options};

/// Upstream's `--help`, byte for byte.
const HELP: &str = include_str!("elm_format_help.txt");

/// Why parsing stopped short of options.
#[derive(Debug, PartialEq, Eq)]
enum Stop {
    Help,
    Error(String),
}

pub(crate) fn main(args: impl Iterator<Item = OsString>) -> ExitCode {
    let options = match parse(args) {
        Ok(options) => options,
        Err(Stop::Help) => return help(),
        Err(Stop::Error(message)) => {
            let _ = write!(io::stderr(), "{message}\n\n{HELP}");
            return ExitCode::FAILURE;
        }
    };
    match elm_format::run(&Vfs::new(), &options, &mut StdConsole) {
        Exit::Success => ExitCode::SUCCESS,
        Exit::Failure => ExitCode::FAILURE,
        Exit::Usage => help(),
    }
}

fn help() -> ExitCode {
    let _ = io::stdout().write_all(HELP.as_bytes());
    ExitCode::SUCCESS
}

/// The options `args` give, read left to right until the first help
/// request or error. Options take no abbreviations and none may repeat;
/// `--name=value` and `--name value` both work, and after `--` every
/// argument is an INPUT.
fn parse(args: impl Iterator<Item = OsString>) -> Result<Options, Stop> {
    let mut options = Options::default();
    let mut args = args.peekable();
    let mut positional_only = false;
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy().into_owned();
        if positional_only || text == "-" || !text.starts_with('-') {
            options.inputs.push(PathBuf::from(arg));
            continue;
        }
        if text == "--" {
            positional_only = true;
            continue;
        }
        let invalid = || Stop::Error(format!("Invalid option `{text}'"));
        let Some(long) = text.strip_prefix("--") else {
            // `-h` is the only short option, and opens a bundle.
            return Err(if text.starts_with("-h") {
                Stop::Help
            } else {
                invalid()
            });
        };
        let (name, inline) = match long.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (long, None),
        };
        match name {
            // Upstream ignores a value given to `--help`.
            "help" => return Err(Stop::Help),
            "yes" | "validate" | "stdin" => {
                let flag = match name {
                    "yes" => &mut options.yes,
                    "validate" => &mut options.validate,
                    _ => &mut options.stdin,
                };
                if inline.is_some() || *flag {
                    return Err(invalid());
                }
                *flag = true;
            }
            "output" | "elm-version" => {
                let taken = match name {
                    "output" => options.output.is_some(),
                    _ => options.elm_version.is_some(),
                };
                if taken {
                    return Err(invalid());
                }
                let value = match inline {
                    Some(value) => value,
                    None => match args.next() {
                        Some(value) => value.to_string_lossy().into_owned(),
                        None => {
                            return Err(Stop::Error(format!(
                                "The option `--{name}` expects an argument."
                            )));
                        }
                    },
                };
                if name == "output" {
                    options.output = Some(PathBuf::from(value));
                } else {
                    options.elm_version = Some(elm_version(&value)?);
                }
            }
            _ => return Err(invalid()),
        }
    }
    // optparse-applicative's `many` collects arguments last first.
    options.inputs.reverse();
    Ok(options)
}

fn elm_version(value: &str) -> Result<ElmVersion, Stop> {
    match value {
        "0.17" => Ok(ElmVersion::V0_17),
        "0.18" => Ok(ElmVersion::V0_18),
        "0.19" => Ok(ElmVersion::V0_19),
        _ => Err(Stop::Error(format!(
            "option --elm-version: Invalid Elm version \"{value}\".  Supported versions are 0.18, 0.19"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Result<Options, Stop> {
        parse(args.iter().map(OsString::from))
    }

    fn error(args: &[&str]) -> String {
        match parsed(args) {
            Err(Stop::Error(message)) => message,
            other => panic!("{args:?} parsed as {other:?}"),
        }
    }

    #[test]
    fn inputs_come_last_first() {
        let options = parsed(&["a", "--yes", "b", "--", "--c", "-"]).unwrap();
        assert_eq!(options.inputs, ["-", "--c", "b", "a"].map(PathBuf::from));
        assert!(options.yes);
    }

    #[test]
    fn values_follow_or_are_inline() {
        let options = parsed(&["--output", "--yes", "--elm-version=0.19"]).unwrap();
        assert_eq!(options.output, Some(PathBuf::from("--yes")));
        assert_eq!(options.elm_version, Some(ElmVersion::V0_19));
        assert!(!options.yes);
    }

    #[test]
    fn the_first_help_or_error_wins() {
        assert_eq!(parsed(&["-hx", "--bad"]).unwrap_err(), Stop::Help);
        assert_eq!(parsed(&["x", "--help=1", "--bad"]).unwrap_err(), Stop::Help);
        assert_eq!(error(&["--bad", "--help"]), "Invalid option `--bad'");
    }

    #[test]
    fn errors_quote_the_argument() {
        assert_eq!(error(&["-xh"]), "Invalid option `-xh'");
        assert_eq!(error(&["--he"]), "Invalid option `--he'");
        assert_eq!(error(&["--yes=1"]), "Invalid option `--yes=1'");
        assert_eq!(error(&["---"]), "Invalid option `---'");
        assert_eq!(error(&["--stdin", "--stdin"]), "Invalid option `--stdin'");
        assert_eq!(
            error(&["--elm-version=0.19", "--elm-version=0.19"]),
            "Invalid option `--elm-version=0.19'"
        );
        assert_eq!(
            error(&["--output"]),
            "The option `--output` expects an argument."
        );
        assert_eq!(
            error(&["--elm-version", ""]),
            "option --elm-version: Invalid Elm version \"\".  Supported versions are 0.18, 0.19"
        );
    }
}
