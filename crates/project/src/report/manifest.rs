//! What a bad `nomi.toml` says: the problem, then the lines it is about
//! with its bytes marked, as a bad `elm.json` shows them.

use nomi_base::diagnostic::code::Source;
use nomi_base::diagnostic::doc::reflow;
use nomi_base::{Compat, Report};

use crate::manifest::{ManifestError, ManifestProblem};

/// The report of `error`, found reading `src`, with positions as
/// `compat` keeps them.
pub fn manifest(error: &ManifestError, src: &[u8], compat: Compat) -> Report {
    let (title, problem, hint) = match &error.problem {
        ManifestProblem::Syntax(message) => (
            "BAD TOML",
            "It is not valid TOML here:".to_owned(),
            format!("The TOML reader says: {message}."),
        ),
        ManifestProblem::UnknownKey { key, known } => (
            "UNKNOWN KEY",
            format!("I do not know the `{key}` key here:"),
            format!("I know {} here.", listing(known)),
        ),
        ManifestProblem::WrongType {
            key,
            expected,
            found,
        } => (
            "WRONG TYPE",
            format!(
                "The `{key}` key needs {}, but this is {}:",
                a(expected),
                a(found)
            ),
            how_to_write(key).to_owned(),
        ),
        ManifestProblem::UnknownPlatform(name) => (
            "UNKNOWN PLATFORM",
            format!("I do not know the \"{name}\" platform:"),
            "The platforms are \"node\" and \"browser\". Without one, an entry runs on Node \
             when its `main` is a Nomi.Program, and in the browser otherwise."
                .to_owned(),
        ),
        ManifestProblem::MissingMain => (
            "MISSING MAIN",
            "This entry has no `main`:".to_owned(),
            "Every entry names the Elm module it starts from, as in main = \"src/Main.elm\"."
                .to_owned(),
        ),
    };
    let text = nomi_base::ghc::lenient_utf8(src);
    let source = Source::new(&text);
    let before = reflow(&format!(
        "I ran into a problem with your nomi.toml file. {problem}"
    ));
    let region = source.region(error.range, compat);
    Report {
        title: title.into(),
        path: Some("nomi.toml".to_owned()),
        message: source.snippet(compat, before, region, None, reflow(&hint)),
    }
}

/// `word` after its indefinite article.
fn a(word: &str) -> String {
    let article = if word.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    };
    format!("{article} {word}")
}

/// `keys` quoted, the last after "and".
fn listing(keys: &[&str]) -> String {
    match keys {
        [] => String::new(),
        [key] => format!("`{key}`"),
        [init @ .., last] => {
            let init: Vec<String> = init.iter().map(|key| format!("`{key}`")).collect();
            format!("{} and `{last}`", init.join(", "))
        }
    }
}

/// How `key`'s value is written.
fn how_to_write(key: &str) -> &'static str {
    match key {
        "main" => "For example: main = \"src/Main.elm\"",
        "output" => "For example: output = \"dist/main.mjs\"",
        "platform" => "For example: platform = \"node\"",
        "ffi" => "For example: ffi = \"src/ffi.mjs\"",
        _ => "Each entry is a table of its own, under an [[entry]] header.",
    }
}
