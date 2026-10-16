//! `nomi.toml`: what elm.json can't say, the entries a project builds,
//! each a `main` with its output, platform and FFI module, and at a
//! workspace's root, its members. Read strictly: a key nomi doesn't know
//! is an error, so a typo never passes silently.

use nomi_base::{Platform, TextRange, TextSize};
use toml::Spanned;
use toml::de::{DeTable, DeValue};

use crate::workspace::Pattern;

/// A project's `nomi.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    /// Its `[workspace]` table, at a workspace's root.
    pub workspace: Option<Workspace>,
    /// Its `[[entry]]` tables, in order.
    pub entries: Vec<Entry>,
}

/// A workspace, from a `[workspace]` table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Workspace {
    /// Its `members`, in order.
    pub members: Vec<Member>,
}

/// One of a workspace's `members`, and the bytes it was written in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub pattern: Pattern,
    pub range: TextRange,
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
    /// The arguments `nomi run` gives a Node program when it runs every
    /// entry.
    pub args: Vec<String>,
    /// The directory a browser program's server serves at `/`.
    pub static_dir: Option<String>,
    /// The port a browser program's server listens on.
    pub port: Option<u16>,
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
    /// A workspace member that is empty, absolute, has `..`, or uses `*` in
    /// part of a component.
    BadMember(String),
    /// A port outside 1 to 65535.
    PortRange,
    /// A key that an entry on `platform` can't use.
    WrongPlatformKey {
        key: &'static str,
        platform: Platform,
    },
}

/// The keys of the file itself.
const TOP: &[&str] = &["workspace", "entry"];

/// The keys of a workspace.
const WORKSPACE: &[&str] = &["members"];

/// The keys of an entry.
const ENTRY: &[&str] = &[
    "main", "output", "platform", "ffi", "args", "static", "port",
];

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
        let mut manifest = Manifest::default();
        for (key, value) in fields(root.get_ref(), TOP)? {
            if key == "workspace" {
                let Some(table) = value.get_ref().as_table() else {
                    return Err(wrong_type(key, "table", value));
                };
                manifest.workspace = Some(workspace(table)?);
                continue;
            }
            let Some(items) = value.get_ref().as_array() else {
                return Err(wrong_type(key, "array of tables", value));
            };
            for item in items {
                let Some(table) = item.get_ref().as_table() else {
                    return Err(wrong_type(key, "table", item));
                };
                manifest.entries.push(entry(table, item.span())?);
            }
        }
        Ok(manifest)
    }
}

/// The workspace `table`.
fn workspace(table: &DeTable<'_>) -> Result<Workspace, ManifestError> {
    let mut members = Vec::new();
    for (key, value) in fields(table, WORKSPACE)? {
        for item in strings(key, value)? {
            let text = string(key, item)?;
            let range = range(item.span());
            let pattern = Pattern::parse(text).ok_or_else(|| ManifestError {
                range,
                problem: ManifestProblem::BadMember(text.to_owned()),
            })?;
            members.push(Member { pattern, range });
        }
    }
    Ok(Workspace { members })
}

/// The entry `table`, whose header is at `header`.
fn entry(table: &DeTable<'_>, header: std::ops::Range<usize>) -> Result<Entry, ManifestError> {
    let mut main = None;
    let mut output = None;
    let mut platform = None;
    let mut ffi = None;
    let mut args = Vec::new();
    let mut static_dir = None;
    let mut port = None;
    // The first key only Node can use and the first only a browser can,
    // refused once the entry's platform is known to be the other.
    let mut node_key = None;
    let mut browser_key = None;
    for (key, value) in fields(table, ENTRY)? {
        match key {
            "args" => {
                node_key.get_or_insert_with(|| (key, value.span()));
                for item in strings(key, value)? {
                    args.push(string(key, item)?.to_owned());
                }
            }
            "port" => {
                browser_key.get_or_insert_with(|| (key, value.span()));
                port = Some(port_number(value)?);
            }
            _ => {
                let text = string(key, value)?;
                match key {
                    "main" => main = Some(text.to_owned()),
                    "output" => output = Some(text.to_owned()),
                    "platform" => platform = Some(parse_platform(text, value)?),
                    "static" => {
                        browser_key.get_or_insert_with(|| (key, value.span()));
                        static_dir = Some(text.to_owned());
                    }
                    _ => ffi = Some(text.to_owned()),
                }
            }
        }
    }
    let other_key = |platform| match platform {
        Platform::Node => browser_key,
        Platform::Browser => node_key,
    };
    if let Some(platform) = platform
        && let Some((key, span)) = other_key(platform)
    {
        return Err(ManifestError {
            range: range(span),
            problem: ManifestProblem::WrongPlatformKey { key, platform },
        });
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
        args,
        static_dir,
        port,
    })
}

/// The port `value`, an integer from 1 to 65535.
fn port_number(value: &Spanned<DeValue<'_>>) -> Result<u16, ManifestError> {
    let Some(number) = value.get_ref().as_integer() else {
        return Err(wrong_type("port", "integer", value));
    };
    u16::from_str_radix(number.as_str(), number.radix())
        .ok()
        .filter(|&port| port > 0)
        .ok_or_else(|| ManifestError {
            range: range(value.span()),
            problem: ManifestProblem::PortRange,
        })
}

/// The platform `text`, the string `value`.
fn parse_platform(text: &str, value: &Spanned<DeValue<'_>>) -> Result<Platform, ManifestError> {
    match text {
        "node" => Ok(Platform::Node),
        "browser" => Ok(Platform::Browser),
        _ => Err(ManifestError {
            range: range(value.span()),
            problem: ManifestProblem::UnknownPlatform(text.to_owned()),
        }),
    }
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

/// The items of `value`, the array of strings of `key`.
fn strings<'v, 'i>(
    key: &'static str,
    value: &'v Spanned<DeValue<'i>>,
) -> Result<&'v [Spanned<DeValue<'i>>], ManifestError> {
    value
        .get_ref()
        .as_array()
        .map(|items| &items[..])
        .ok_or_else(|| wrong_type(key, "array of strings", value))
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
