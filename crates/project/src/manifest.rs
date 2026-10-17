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
    pub workspace: Option<WorkspaceTable>,
    /// Its `[[entry]]` tables, in order.
    pub entries: Vec<Entry>,
}

/// A `[workspace]` table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceTable {
    /// Its `members`, in order.
    pub members: Vec<MemberPattern>,
}

/// One of a workspace's `members`, and the bytes it was written in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberPattern {
    pub pattern: Pattern,
    pub range: TextRange,
}

/// A key of a `nomi.toml`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Workspace,
    Entry,
    Members,
    Main,
    Output,
    Platform,
    Ffi,
    Args,
    Static,
    Port,
}

impl Key {
    /// The key as the file writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Key::Workspace => "workspace",
            Key::Entry => "entry",
            Key::Members => "members",
            Key::Main => "main",
            Key::Output => "output",
            Key::Platform => "platform",
            Key::Ffi => "ffi",
            Key::Args => "args",
            Key::Static => "static",
            Key::Port => "port",
        }
    }
}

/// A type of TOML value, as the TOML reader names it, or what a key needs
/// of one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    String,
    Integer,
    Float,
    Boolean,
    Datetime,
    Array,
    Table,
    ArrayOfStrings,
    ArrayOfTables,
}

impl ValueType {
    /// The type of `value`.
    fn of(value: &DeValue<'_>) -> ValueType {
        match value {
            DeValue::String(_) => ValueType::String,
            DeValue::Integer(_) => ValueType::Integer,
            DeValue::Float(_) => ValueType::Float,
            DeValue::Boolean(_) => ValueType::Boolean,
            DeValue::Datetime(_) => ValueType::Datetime,
            DeValue::Array(_) => ValueType::Array,
            DeValue::Table(_) => ValueType::Table,
        }
    }

    /// Its name, as the TOML reader's `type_str` gives it.
    pub fn as_str(self) -> &'static str {
        match self {
            ValueType::String => "string",
            ValueType::Integer => "integer",
            ValueType::Float => "float",
            ValueType::Boolean => "boolean",
            ValueType::Datetime => "datetime",
            ValueType::Array => "array",
            ValueType::Table => "table",
            ValueType::ArrayOfStrings => "array of strings",
            ValueType::ArrayOfTables => "array of tables",
        }
    }
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

/// What is wrong with a `nomi.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestProblem {
    /// Not UTF-8 or not TOML, with the TOML reader's message.
    Syntax(String),
    /// A key its table doesn't have, which has the `known` ones.
    UnknownKey {
        key: String,
        known: &'static [Key],
    },
    /// A value of the wrong type for `key`.
    WrongType {
        key: Key,
        expected: ValueType,
        found: ValueType,
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
        key: Key,
        platform: Platform,
    },
    /// A member, written as the directory named, with no `elm.json` in
    /// that directory.
    MissingMember(String),
}

/// The keys of the file itself.
const TOP: &[Key] = &[Key::Workspace, Key::Entry];

/// The keys of a workspace.
const WORKSPACE: &[Key] = &[Key::Members];

/// The keys of an entry.
const ENTRY: &[Key] = &[
    Key::Main,
    Key::Output,
    Key::Platform,
    Key::Ffi,
    Key::Args,
    Key::Static,
    Key::Port,
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
            match key {
                Key::Workspace => {
                    let Some(table) = value.get_ref().as_table() else {
                        return Err(wrong_type(key, ValueType::Table, value));
                    };
                    manifest.workspace = Some(workspace(table)?);
                }
                Key::Entry => {
                    let Some(items) = value.get_ref().as_array() else {
                        return Err(wrong_type(key, ValueType::ArrayOfTables, value));
                    };
                    for item in items {
                        let Some(table) = item.get_ref().as_table() else {
                            return Err(wrong_type(key, ValueType::Table, item));
                        };
                        manifest.entries.push(entry(table, item.span())?);
                    }
                }
                Key::Members
                | Key::Main
                | Key::Output
                | Key::Platform
                | Key::Ffi
                | Key::Args
                | Key::Static
                | Key::Port => unreachable!("the file's keys are among TOP"),
            }
        }
        Ok(manifest)
    }
}

/// The workspace `table`.
fn workspace(table: &DeTable<'_>) -> Result<WorkspaceTable, ManifestError> {
    let mut members = Vec::new();
    for (key, value) in fields(table, WORKSPACE)? {
        for item in strings(key, value)? {
            let text = string(key, item)?;
            let range = range(item.span());
            let pattern = Pattern::parse(text).ok_or_else(|| ManifestError {
                range,
                problem: ManifestProblem::BadMember(text.to_owned()),
            })?;
            members.push(MemberPattern { pattern, range });
        }
    }
    Ok(WorkspaceTable { members })
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
            Key::Args => {
                node_key.get_or_insert_with(|| (key, value.span()));
                for item in strings(key, value)? {
                    args.push(string(key, item)?.to_owned());
                }
            }
            Key::Port => {
                browser_key.get_or_insert_with(|| (key, value.span()));
                port = Some(port_number(value)?);
            }
            Key::Main => main = Some(string(key, value)?.to_owned()),
            Key::Output => output = Some(string(key, value)?.to_owned()),
            Key::Platform => platform = Some(parse_platform(string(key, value)?, value)?),
            Key::Static => {
                browser_key.get_or_insert_with(|| (key, value.span()));
                static_dir = Some(string(key, value)?.to_owned());
            }
            Key::Ffi => {
                node_key.get_or_insert_with(|| (key, value.span()));
                ffi = Some(string(key, value)?.to_owned());
            }
            Key::Workspace | Key::Entry | Key::Members => {
                unreachable!("an entry's keys are among ENTRY")
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
        return Err(wrong_type(Key::Port, ValueType::Integer, value));
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
    known: &'static [Key],
) -> Result<Vec<(Key, &'t Spanned<DeValue<'i>>)>, ManifestError> {
    // The map may keep its keys sorted rather than in order.
    let mut fields: Vec<_> = table.iter().collect();
    fields.sort_by_key(|(key, _)| key.span().start);
    fields
        .into_iter()
        .map(
            |(key, value)| match known.iter().find(|name| name.as_str() == key.get_ref()) {
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
    key: Key,
    value: &'v Spanned<DeValue<'i>>,
) -> Result<&'v [Spanned<DeValue<'i>>], ManifestError> {
    value
        .get_ref()
        .as_array()
        .map(|items| &items[..])
        .ok_or_else(|| wrong_type(key, ValueType::ArrayOfStrings, value))
}

/// The string `value` of `key`.
fn string<'v>(key: Key, value: &'v Spanned<DeValue<'_>>) -> Result<&'v str, ManifestError> {
    value
        .get_ref()
        .as_str()
        .ok_or_else(|| wrong_type(key, ValueType::String, value))
}

fn wrong_type(key: Key, expected: ValueType, value: &Spanned<DeValue<'_>>) -> ManifestError {
    ManifestError {
        range: range(value.span()),
        problem: ManifestProblem::WrongType {
            key,
            expected,
            found: ValueType::of(value.get_ref()),
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
