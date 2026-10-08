//! `elm.json` as upstream reads and writes it (`Elm/Outline.hs`): an
//! application's or a package's outline, decoded with upstream's decoders
//! in upstream's order so that a bad file fails with upstream's problem,
//! and encoded back in upstream's layout.
//!
//! Reading is pure here. Upstream goes on to check that an application's
//! source directories exist and are distinct; that needs the file system,
//! so callers do it.

use std::collections::BTreeMap;

use nomi_base::Compat;

use crate::json::decode::{self, Alternative, Decoded, Problem};
use crate::json::{self, ParseError, Position, Value, encode};
use crate::{Constraint, ConstraintError, License, ModuleName, Name, Version, scan};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outline {
    App(AppOutline),
    Pkg(PkgOutline),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppOutline {
    pub elm: Version,
    /// Never empty.
    pub source_dirs: Vec<SrcDir>,
    pub direct: BTreeMap<Name, Version>,
    pub indirect: BTreeMap<Name, Version>,
    pub test_direct: BTreeMap<Name, Version>,
    pub test_indirect: BTreeMap<Name, Version>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PkgOutline {
    pub name: Name,
    /// The raw string bytes, escapes as written.
    pub summary: Box<[u8]>,
    pub license: License,
    pub version: Version,
    pub exposed: Exposed,
    pub deps: BTreeMap<Name, Constraint>,
    pub test_deps: BTreeMap<Name, Constraint>,
    pub elm: Constraint,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exposed {
    List(Vec<ModuleName>),
    /// Sections in source order, each under its raw header bytes.
    Dict(Vec<(Box<[u8]>, Vec<ModuleName>)>),
}

impl Exposed {
    /// `flattenExposed`: every exposed module, in order.
    pub fn modules(&self) -> Box<dyn Iterator<Item = &ModuleName> + '_> {
        match self {
            Exposed::List(names) => Box::new(names.iter()),
            Exposed::Dict(sections) => Box::new(sections.iter().flat_map(|(_, names)| names)),
        }
    }
}

/// A source directory as written between its quotes. Upstream reads it
/// with `Json.toChars`, which decodes no escapes, so `"src\\lib"` names a
/// directory with two backslashes in it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SrcDir(Box<[u8]>);

impl SrcDir {
    pub fn raw(&self) -> &[u8] {
        &self.0
    }

    /// `Json.toChars`: the characters upstream makes of the bytes. Strings
    /// from 0.19.2 on are valid UTF-8; 0.19.1's may not be, and it then
    /// assembles characters unchecked, so a code may be no character at
    /// all, even negative.
    pub fn chars(&self) -> Vec<i64> {
        scan::unchecked_chars(&self.0)
    }

    /// The path upstream uses, if its characters are characters.
    pub fn path(&self) -> Option<String> {
        self.chars()
            .into_iter()
            .map(|code| u32::try_from(code).ok().and_then(char::from_u32))
            .collect()
    }

    /// `encodePath`: `JE.chars` of the characters, which escapes four of
    /// them and writes each with `charUtf8`. Backslashes double on every
    /// rewrite, as upstream's do.
    fn encode(&self) -> encode::Value {
        let mut out = Vec::with_capacity(self.0.len());
        for code in self.chars() {
            match code {
                0x0D => out.extend_from_slice(b"\\r"),
                0x0A => out.extend_from_slice(b"\\n"),
                0x22 => out.extend_from_slice(b"\\\""),
                0x5C => out.extend_from_slice(b"\\\\"),
                _ => char_utf8(code, &mut out),
            }
        }
        encode::Value::String(out)
    }
}

/// `encodeDeps`: names to versions or constraints, in name order.
fn encode_deps<T: std::fmt::Display>(deps: &BTreeMap<Name, T>) -> encode::Value {
    encode::Value::Object(
        deps.iter()
            .map(|(name, v)| {
                (
                    name.to_string().into_bytes(),
                    encode::Value::chars(v.to_string().as_bytes()),
                )
            })
            .collect(),
    )
}

/// bytestring's `charUtf8` on a GHC `Char` of this code, unchecked: the
/// width comes from the code's size and each byte is truncated, so
/// valid UTF-8 comes back as it was and anything else comes back mangled
/// as upstream mangles it.
fn char_utf8(code: i64, out: &mut Vec<u8>) {
    let byte = |n: i64| n as u8;
    if code <= 0x7F {
        out.push(byte(code));
    } else if code <= 0x7FF {
        out.extend([byte((code >> 6) + 0xC0), byte((code & 0x3F) + 0x80)]);
    } else if code <= 0xFFFF {
        out.extend([
            byte((code >> 12) + 0xE0),
            byte(((code >> 6) & 0x3F) + 0x80),
            byte((code & 0x3F) + 0x80),
        ]);
    } else {
        out.extend([
            byte((code >> 18) + 0xF0),
            byte(((code >> 12) & 0x3F) + 0x80),
            byte(((code >> 6) & 0x3F) + 0x80),
            byte((code & 0x3F) + 0x80),
        ]);
    }
}

/// `Exit.OutlineProblem`: what a field's own decoder rejects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutlineProblem {
    /// `type` is neither `application` nor `package`, as raw bytes.
    BadType,
    BadPkgName(Position),
    BadVersion(Position),
    BadConstraint(ConstraintError),
    BadModuleName(Position),
    /// An `exposed-modules` section header of 20 bytes or more.
    BadModuleHeaderTooLong,
    BadDependencyName(Position),
    /// The raw string and the four nearest licenses.
    BadLicense(Box<[u8]>, Vec<&'static str>),
    /// A summary of 80 bytes or more, escapes as written.
    BadSummaryTooLong,
    NoSrcDirs,
}

/// Why `elm.json` is no outline: `Exit.Outline` without the checks that
/// need the file system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutlineError {
    /// Not JSON as upstream parses it (`OutlineHasBadStructure`).
    Parse(ParseError),
    /// JSON, but not an outline (`OutlineHasBadStructure`).
    Decode(Problem<OutlineProblem>),
    /// A package other than elm/core that doesn't depend on it.
    NoPkgCore,
    /// An application without elm/core among its direct dependencies.
    NoAppCore,
    /// An application without elm/json among its dependencies.
    NoAppJson,
}

impl Outline {
    /// `read`, short of the source directories: the outline in `src`, or
    /// the error `compat`'s elm reports for it.
    pub fn read(src: &[u8], compat: Compat) -> Result<Outline, OutlineError> {
        let value = json::parse(src, compat).map_err(OutlineError::Parse)?;
        let outline = decoder(&value, compat).map_err(OutlineError::Decode)?;
        match &outline {
            Outline::Pkg(pkg)
                if !pkg.deps.contains_key(&Name::core()) && pkg.name != Name::core() =>
            {
                Err(OutlineError::NoPkgCore)
            }
            Outline::App(app) if !app.direct.contains_key(&Name::core()) => {
                Err(OutlineError::NoAppCore)
            }
            Outline::App(app)
                if !app.direct.contains_key(&Name::json())
                    && !app.indirect.contains_key(&Name::json()) =>
            {
                Err(OutlineError::NoAppJson)
            }
            _ => Ok(outline),
        }
    }

    /// `write`'s bytes: the file upstream writes for this outline.
    pub fn write(&self) -> Vec<u8> {
        encode::write(&self.encode())
    }

    /// `encode`.
    pub fn encode(&self) -> encode::Value {
        use encode::Value as V;
        let chars = |text: &dyn std::fmt::Display| V::chars(text.to_string().as_bytes());
        let field = |key: &str, value: V| (key.as_bytes().to_vec(), value);
        match self {
            Outline::App(app) => V::Object(vec![
                field("type", V::chars(b"application")),
                field(
                    "source-directories",
                    V::Array(app.source_dirs.iter().map(SrcDir::encode).collect()),
                ),
                field("elm-version", chars(&app.elm)),
                field(
                    "dependencies",
                    V::Object(vec![
                        field("direct", encode_deps(&app.direct)),
                        field("indirect", encode_deps(&app.indirect)),
                    ]),
                ),
                field(
                    "test-dependencies",
                    V::Object(vec![
                        field("direct", encode_deps(&app.test_direct)),
                        field("indirect", encode_deps(&app.test_indirect)),
                    ]),
                ),
            ]),
            Outline::Pkg(pkg) => {
                let modules = |names: &[ModuleName]| {
                    V::Array(names.iter().map(|n| V::string(n.as_bytes())).collect())
                };
                let exposed = match &pkg.exposed {
                    Exposed::List(names) => modules(names),
                    Exposed::Dict(sections) => V::Object(
                        sections
                            .iter()
                            .map(|(header, names)| (header.to_vec(), modules(names)))
                            .collect(),
                    ),
                };
                V::Object(vec![
                    field("type", V::string(*b"package")),
                    field("name", chars(&pkg.name)),
                    field("summary", V::string(pkg.summary.clone())),
                    field("license", V::string(pkg.license.as_bytes())),
                    field("version", chars(&pkg.version)),
                    field("exposed-modules", exposed),
                    field("elm-version", chars(&pkg.elm)),
                    field("dependencies", encode_deps(&pkg.deps)),
                    field("test-dependencies", encode_deps(&pkg.test_deps)),
                ])
            }
        }
    }
}

fn decoder(value: &Value<'_>, compat: Compat) -> Decoded<Outline, OutlineProblem> {
    let tipe = decode::field(value, b"type", decode::string)?;
    match tipe {
        b"application" => app(value, compat).map(Outline::App),
        b"package" => pkg(value, compat).map(Outline::Pkg),
        _ => decode::failure(value, OutlineProblem::BadType),
    }
}

fn app(value: &Value<'_>, compat: Compat) -> Decoded<AppOutline, OutlineProblem> {
    let versions = |v: &Value<'_>| deps(v, compat, |v| version(v, compat));
    let deps_field = |outer: &[u8], inner: &[u8]| {
        decode::field(value, outer, |v| decode::field(v, inner, versions))
    };
    Ok(AppOutline {
        elm: decode::field(value, b"elm-version", |v| version(v, compat))?,
        source_dirs: decode::field(value, b"source-directories", |v| {
            decode::non_empty_list(
                v,
                |e| decode::string(e).map(|raw| SrcDir(raw.into())),
                OutlineProblem::NoSrcDirs,
            )
        })?,
        direct: deps_field(b"dependencies", b"direct")?,
        indirect: deps_field(b"dependencies", b"indirect")?,
        test_direct: deps_field(b"test-dependencies", b"direct")?,
        test_indirect: deps_field(b"test-dependencies", b"indirect")?,
    })
}

fn pkg(value: &Value<'_>, compat: Compat) -> Decoded<PkgOutline, OutlineProblem> {
    let constraints = |v: &Value<'_>| deps(v, compat, |v| constraint(v, compat));
    Ok(PkgOutline {
        name: decode::field(value, b"name", |v| {
            decode::custom_string(v, |bytes, start| {
                Name::parse(bytes, start, compat).map_err(OutlineProblem::BadPkgName)
            })
        })?,
        summary: decode::field(value, b"summary", |v| {
            decode::custom_string(v, |bytes, _| {
                bounded(bytes, 80, OutlineProblem::BadSummaryTooLong)
            })
        })?,
        license: decode::field(value, b"license", |v| {
            let raw = decode::string(v)?;
            License::check(raw, compat).or_else(|nearest| {
                decode::failure(v, OutlineProblem::BadLicense(raw.into(), nearest))
            })
        })?,
        version: decode::field(value, b"version", |v| version(v, compat))?,
        exposed: decode::field(value, b"exposed-modules", |v| exposed(v, compat))?,
        deps: decode::field(value, b"dependencies", constraints)?,
        test_deps: decode::field(value, b"test-dependencies", constraints)?,
        elm: decode::field(value, b"elm-version", |v| constraint(v, compat))?,
    })
}

fn version(value: &Value<'_>, compat: Compat) -> Decoded<Version, OutlineProblem> {
    decode::custom_string(value, |bytes, start| {
        Version::parse(bytes, start, compat).map_err(OutlineProblem::BadVersion)
    })
}

fn constraint(value: &Value<'_>, compat: Compat) -> Decoded<Constraint, OutlineProblem> {
    decode::custom_string(value, |bytes, start| {
        Constraint::parse(bytes, start, compat).map_err(OutlineProblem::BadConstraint)
    })
}

fn module(value: &Value<'_>, compat: Compat) -> Decoded<ModuleName, OutlineProblem> {
    decode::custom_string(value, |bytes, start| {
        ModuleName::parse(bytes, start, compat).map_err(OutlineProblem::BadModuleName)
    })
}

/// `depsDecoder`: names to values, in name order, the last of equal names
/// winning.
fn deps<'a, T>(
    value: &Value<'a>,
    compat: Compat,
    entry: impl FnMut(&Value<'a>) -> Decoded<T, OutlineProblem>,
) -> Decoded<BTreeMap<Name, T>, OutlineProblem> {
    decode::dict(
        value,
        |bytes, start| Name::parse(bytes, start, compat).map_err(OutlineProblem::BadDependencyName),
        entry,
    )
}

/// `exposedDecoder`: a list of modules, or else an object of sections.
fn exposed(value: &Value<'_>, compat: Compat) -> Decoded<Exposed, OutlineProblem> {
    let modules = |v: &Value<'_>| decode::list(v, |e| module(e, compat));
    let mut list = |v: &Value<'_>| modules(v).map(Exposed::List);
    let mut dict = |v: &Value<'_>| {
        decode::pairs(
            v,
            |bytes, _| bounded(bytes, 20, OutlineProblem::BadModuleHeaderTooLong),
            modules,
        )
        .map(Exposed::Dict)
    };
    let alternatives: &mut [&mut Alternative<'_, '_, Exposed, OutlineProblem>] =
        &mut [&mut list, &mut dict];
    decode::one_of(value, alternatives)
}

/// `boundParser`: the raw bytes if there are fewer than `bound` of them.
fn bounded(
    bytes: &[u8],
    bound: usize,
    too_long: OutlineProblem,
) -> Result<Box<[u8]>, OutlineProblem> {
    if bytes.len() < bound {
        Ok(bytes.into())
    } else {
        Err(too_long)
    }
}
