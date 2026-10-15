//! `nomi.toml`: what elm.json can't say, the entries a project builds,
//! each a `main` with its output, platform and FFI module. Read strictly:
//! a key nomi doesn't know is an error, so a typo never passes silently.

use nomi_base::{Platform, TextRange, TextSize};
use toml::Spanned;
use toml::de::{DeTable, DeValue};

/// A project's `nomi.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    /// Its `[[entry]]` tables, in order.
    pub entries: Vec<Entry>,
}

/// A program the project builds, from an `[[entry]]` table. Paths are as
/// written, relative to the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The Elm module whose `main` the program runs.
    pub main: String,
    pub output: Option<String>,
    /// Where the program runs, unless `main`'s annotation decides.
    pub platform: Option<Platform>,
    /// The JS module the program's `Ffi.task`s call.
    pub ffi: Option<String>,
}

/// Why a `nomi.toml` was refused, and the bytes it is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestError {
    pub range: TextRange,
    pub problem: ManifestProblem,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestProblem {
    /// Not UTF-8 or not TOML, with the TOML reader's message.
    Syntax(String),
    /// A key its table doesn't have, which has the `known` ones.
    UnknownKey {
        key: String,
        known: &'static [&'static str],
    },
    /// A value of the wrong type for `key`, by TOML's names of types.
    WrongType {
        key: &'static str,
        expected: &'static str,
        found: &'static str,
    },
    UnknownPlatform(String),
    /// An entry without `main`, at its header.
    MissingMain,
}

/// The keys of the file itself.
const TOP: &[&str] = &["entry"];

/// The keys of an entry.
const ENTRY: &[&str] = &["main", "output", "platform", "ffi"];

impl Manifest {
    /// Reads `src`, a `nomi.toml`.
    pub fn parse(src: &[u8]) -> Result<Manifest, ManifestError> {
        let text = std::str::from_utf8(src).map_err(|e| ManifestError {
            range: range(e.valid_up_to()..e.valid_up_to()),
            problem: ManifestProblem::Syntax("invalid UTF-8".to_owned()),
        })?;
        let root = DeTable::parse(text).map_err(|e| ManifestError {
            range: range(e.span().unwrap_or_default()),
            problem: ManifestProblem::Syntax(e.message().to_owned()),
        })?;
        let mut entries = Vec::new();
        for (key, value) in fields(root.get_ref(), TOP)? {
            let Some(items) = value.get_ref().as_array() else {
                return Err(wrong_type(key, "array of tables", value));
            };
            for item in items {
                let Some(table) = item.get_ref().as_table() else {
                    return Err(wrong_type(key, "table", item));
                };
                entries.push(entry(table, item.span())?);
            }
        }
        Ok(Manifest { entries })
    }
}

/// The entry `table`, whose header is at `header`.
fn entry(table: &DeTable<'_>, header: std::ops::Range<usize>) -> Result<Entry, ManifestError> {
    let mut main = None;
    let mut output = None;
    let mut platform = None;
    let mut ffi = None;
    for (key, value) in fields(table, ENTRY)? {
        let text = string(key, value)?;
        match key {
            "main" => main = Some(text.to_owned()),
            "output" => output = Some(text.to_owned()),
            "platform" => {
                platform = Some(match text {
                    "node" => Platform::Node,
                    "browser" => Platform::Browser,
                    _ => {
                        return Err(ManifestError {
                            range: range(value.span()),
                            problem: ManifestProblem::UnknownPlatform(text.to_owned()),
                        });
                    }
                });
            }
            _ => ffi = Some(text.to_owned()),
        }
    }
    let main = main.ok_or_else(|| ManifestError {
        range: range(header),
        problem: ManifestProblem::MissingMain,
    })?;
    Ok(Entry {
        main,
        output,
        platform,
        ffi,
    })
}

/// The keys and values of `table` in the file's order, each key as one of
/// `known`; the first key not among them is an error.
fn fields<'t, 'i>(
    table: &'t DeTable<'i>,
    known: &'static [&'static str],
) -> Result<Vec<(&'static str, &'t Spanned<DeValue<'i>>)>, ManifestError> {
    // The map may keep its keys sorted rather than in order.
    let mut fields: Vec<_> = table.iter().collect();
    fields.sort_by_key(|(key, _)| key.span().start);
    fields
        .into_iter()
        .map(
            |(key, value)| match known.iter().find(|&&name| name == key.get_ref()) {
                Some(&name) => Ok((name, value)),
                None => Err(ManifestError {
                    range: range(key.span()),
                    problem: ManifestProblem::UnknownKey {
                        key: key.get_ref().to_string(),
                        known,
                    },
                }),
            },
        )
        .collect()
}

/// The string `value` of `key`.
fn string<'v>(
    key: &'static str,
    value: &'v Spanned<DeValue<'_>>,
) -> Result<&'v str, ManifestError> {
    value
        .get_ref()
        .as_str()
        .ok_or_else(|| wrong_type(key, "string", value))
}

fn wrong_type(
    key: &'static str,
    expected: &'static str,
    value: &Spanned<DeValue<'_>>,
) -> ManifestError {
    ManifestError {
        range: range(value.span()),
        problem: ManifestProblem::WrongType {
            key,
            expected,
            found: value.get_ref().type_str(),
        },
    }
}

/// `span` as a text range, which ends at 4 GiB.
fn range(span: std::ops::Range<usize>) -> TextRange {
    let size = |offset| TextSize::try_from(offset).unwrap_or(TextSize::new(u32::MAX));
    TextRange::new(size(span.start), size(span.end))
}

#[cfg(test)]
mod tests;
