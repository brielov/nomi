//! `elm.json` as upstream reads and writes it (`Elm/Outline.hs`): an
//! application's or a package's outline, decoded with upstream's decoders
//! in upstream's order so that a bad file fails with upstream's problem,
//! and encoded back in upstream's layout.
//!
//! Reading is pure here. Upstream goes on to check that an application's
//! source directories exist and are distinct; that needs the file system,
//! so callers do it.

use std::collections::BTreeMap;

use nomi_base::{Compat, utf8};

use nomi_base::diagnostic::json::escape;

use crate::json::decode::{self, Decoded, Problem};
use crate::json::{self, Cursor, ParseError, Position, Value, encode};
use crate::{Constraint, ConstraintError, License, ModuleName, Name, Version};

/// What an `elm.json` describes: an application or a package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outline {
    App(AppOutline),
    Pkg(PkgOutline),
}

/// An application's `elm.json`.
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

impl AppOutline {
    /// Every dependency at its version. A name in more than one field,
    /// which upstream rejects, takes its direct version, then its
    /// indirect one, then its direct test one.
    pub fn all_deps(&self) -> BTreeMap<Name, Version> {
        let mut all = self.test_indirect.clone();
        for deps in [&self.test_direct, &self.indirect, &self.direct] {
            all.extend(deps.iter().map(|(name, &version)| (name.clone(), version)));
        }
        all
    }
}

/// A package's `elm.json`.
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

/// A package's `exposed-modules`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exposed {
    List(Vec<ModuleName>),
    /// Sections in source order, each under its raw header bytes.
    Dict(Vec<(Box<[u8]>, Vec<ModuleName>)>),
}

impl Exposed {
    /// `flattenExposed`: every exposed module, in order.
    pub fn modules(&self) -> impl Iterator<Item = &ModuleName> {
        let (list, sections) = match self {
            Exposed::List(names) => (names.as_slice(), [].as_slice()),
            Exposed::Dict(sections) => ([].as_slice(), sections.as_slice()),
        };
        list.iter()
            .chain(sections.iter().flat_map(|(_, names)| names))
    }
}

/// A source directory as written between its quotes. Upstream reads it
/// with `Json.toChars`, which decodes no escapes, so `"src\\lib"` names a
/// directory with two backslashes in it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SrcDir(Box<[u8]>);

impl SrcDir {
    /// The directory at `path`, relative to the project, as upstream
    /// writes `R.Relative` paths.
    pub fn new(path: &str) -> SrcDir {
        SrcDir(path.as_bytes().into())
    }

    /// The bytes between the quotes.
    pub fn raw(&self) -> &[u8] {
        &self.0
    }

    /// The path upstream uses, if its characters are characters: strings
    /// from 0.19.2 on are valid UTF-8, but 0.19.1's may not be, and its
    /// `Json.toChars` then assembles codes that are none.
    pub fn path(&self) -> Option<String> {
        utf8::unchecked_chars(&self.0)
            .map(|code| u32::try_from(code).ok().and_then(char::from_u32))
            .collect()
    }

    /// `encodePath`: `JE.chars` of the characters, which escapes four of
    /// them and writes each with `charUtf8`. Backslashes double on every
    /// rewrite, as upstream's do.
    fn encode(&self) -> encode::Value {
        let mut out = Vec::with_capacity(self.0.len());
        for code in utf8::unchecked_chars(&self.0) {
            match u8::try_from(code).ok().and_then(escape) {
                Some(escaped) => out.extend_from_slice(escaped.as_bytes()),
                None => char_utf8(code, &mut out),
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
    /// `type` is neither `application` nor `package`.
    BadType,
    /// `name` stops being a package name at this position.
    BadPkgName(Position),
    /// A version stops being one at this position.
    BadVersion(Position),
    BadConstraint(ConstraintError),
    /// A module name stops being one at this position.
    BadModuleName(Position),
    /// An `exposed-modules` section header of 20 bytes or more.
    BadModuleHeaderTooLong,
    /// A dependency's name stops being a package name at this position.
    BadDependencyName(Position),
    /// No license by this name: the four nearest.
    BadLicense(Vec<License>),
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

/// Why no supported release of elm builds the project in an `elm.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetectError {
    /// The outline isn't one: the error the elm in `compat` reports.
    Outline(Compat, OutlineError),
    /// An application's `elm-version` that isn't a supported release
    /// (`Exit.DetailsBadElmInAppOutline`).
    AppElm(Version),
    /// A package's `elm-version` that admits no supported release
    /// (`Exit.DetailsBadElmInPkg`).
    PkgElm(Constraint),
}

impl Outline {
    /// The project in `src` and the release whose behavior building it
    /// emulates (rule R5): the `elm-version` of an application, since
    /// upstream builds it only with that release (`Elm/Details.hs`,
    /// `verifyApp`), or the newest release a package's `elm-version`
    /// admits (`verifyPkg`). Each release reads `elm.json` a little
    /// differently, so the outline is read as the release it names.
    ///
    /// Which release that is comes from the first release, newest first,
    /// that reads the file at all. A file none reads names none, so its
    /// error is the newest's.
    pub fn detect(src: &[u8]) -> Result<(Outline, Compat), DetectError> {
        let mut first = None;
        for compat in Compat::ALL.into_iter().rev() {
            match Outline::read(src, compat) {
                Ok(outline) => {
                    let target = outline.compat()?;
                    if target == compat {
                        return Ok((outline, compat));
                    }
                    return Outline::read(src, target)
                        .map(|outline| (outline, target))
                        .map_err(|e| DetectError::Outline(target, e));
                }
                Err(e) => {
                    first.get_or_insert(DetectError::Outline(compat, e));
                }
            }
        }
        Err(first.expect("a release"))
    }

    /// The release this project is built as: see [`Outline::detect`].
    fn compat(&self) -> Result<Compat, DetectError> {
        match self {
            Outline::App(app) => Compat::ALL
                .into_iter()
                .find(|&c| Version::compiler(c) == app.elm)
                .ok_or(DetectError::AppElm(app.elm)),
            Outline::Pkg(pkg) => Compat::ALL
                .into_iter()
                .rev()
                .find(|&c| pkg.elm.satisfies(Version::compiler(c)))
                .ok_or(DetectError::PkgElm(pkg.elm)),
        }
    }

    /// Where the modules are, relative to the project: an application's
    /// source directories whose paths are text ([`SrcDir::path`]), or a
    /// package's `src`.
    pub fn source_dirs(&self) -> Vec<String> {
        match self {
            Outline::App(app) => app.source_dirs.iter().filter_map(SrcDir::path).collect(),
            Outline::Pkg(_) => vec!["src".to_owned()],
        }
    }

    /// The direct dependencies: an application's, or a package's.
    pub fn direct(&self) -> impl Iterator<Item = &Name> {
        let (app, pkg) = match self {
            Outline::App(app) => (Some(app.direct.keys()), None),
            Outline::Pkg(pkg) => (None, Some(pkg.deps.keys())),
        };
        app.into_iter().flatten().chain(pkg.into_iter().flatten())
    }

    /// The direct test dependencies: an application's, or a package's.
    pub fn test_direct(&self) -> impl Iterator<Item = &Name> {
        let (app, pkg) = match self {
            Outline::App(app) => (Some(app.test_direct.keys()), None),
            Outline::Pkg(pkg) => (None, Some(pkg.test_deps.keys())),
        };
        app.into_iter().flatten().chain(pkg.into_iter().flatten())
    }

    /// `read`, short of the source directories: the outline in `src`, or
    /// the error `compat`'s elm reports for it.
    pub fn read(src: &[u8], compat: Compat) -> Result<Outline, OutlineError> {
        let outline = Outline::decode(src, compat)?;
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
            Outline::App(_) | Outline::Pkg(_) => Ok(outline),
        }
    }

    /// `decoder` alone, without `read`'s checks: how the solver reads a
    /// release's `elm.json` (`Deps/Solver.hs`, `constraintsDecoder`).
    pub fn decode(src: &[u8], compat: Compat) -> Result<Outline, OutlineError> {
        let value = json::parse(src, compat).map_err(OutlineError::Parse)?;
        decoder(&value, compat).map_err(OutlineError::Decode)
    }

    /// `write`'s bytes: the file upstream writes for this outline.
    pub fn write(&self) -> Vec<u8> {
        encode::write(&self.encode())
    }

    /// `encode`.
    fn encode(&self) -> encode::Value {
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
                    field("license", V::string(pkg.license.as_str())),
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
    let version = |v: &Value<'_>| custom(v, compat, Version::parse, OutlineProblem::BadVersion);
    let versions = |v: &Value<'_>| deps(v, compat, version);
    let deps_field = |outer: &[u8], inner: &[u8]| {
        decode::field(value, outer, |v| decode::field(v, inner, versions))
    };
    Ok(AppOutline {
        elm: decode::field(value, b"elm-version", version)?,
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
    let constraint =
        |v: &Value<'_>| custom(v, compat, Constraint::parse, OutlineProblem::BadConstraint);
    let constraints = |v: &Value<'_>| deps(v, compat, constraint);
    Ok(PkgOutline {
        name: decode::field(value, b"name", |v| {
            custom(v, compat, Name::parse, OutlineProblem::BadPkgName)
        })?,
        summary: decode::field(value, b"summary", |v| {
            decode::custom_string(v, |bytes, _| {
                bounded(bytes, 80, OutlineProblem::BadSummaryTooLong)
            })
        })?,
        license: decode::field(value, b"license", |v| {
            let raw = decode::string(v)?;
            License::check(raw, compat)
                .or_else(|nearest| decode::failure(v, OutlineProblem::BadLicense(nearest)))
        })?,
        version: decode::field(value, b"version", |v| {
            custom(v, compat, Version::parse, OutlineProblem::BadVersion)
        })?,
        exposed: decode::field(value, b"exposed-modules", |v| exposed(v, compat))?,
        deps: decode::field(value, b"dependencies", constraints)?,
        test_deps: decode::field(value, b"test-dependencies", constraints)?,
        elm: decode::field(value, b"elm-version", constraint)?,
    })
}

/// `customString` with `parse`, its failure the outline's `problem`.
fn custom<T, E>(
    value: &Value<'_>,
    compat: Compat,
    parse: fn(&[u8], Cursor, Compat) -> Result<T, E>,
    problem: fn(E) -> OutlineProblem,
) -> Decoded<T, OutlineProblem> {
    decode::custom_string(value, |bytes, start| {
        parse(bytes, start, compat).map_err(problem)
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

/// `exposedDecoder`: a list of modules, or else an object of sections;
/// failing both, both problems, as `oneOf` gives them.
fn exposed(value: &Value<'_>, compat: Compat) -> Decoded<Exposed, OutlineProblem> {
    let module =
        |e: &Value<'_>| custom(e, compat, ModuleName::parse, OutlineProblem::BadModuleName);
    let modules = |v: &Value<'_>| decode::list(v, module);
    let not_list = match modules(value) {
        Ok(list) => return Ok(Exposed::List(list)),
        Err(problem) => problem,
    };
    decode::pairs(
        value,
        |bytes, _| bounded(bytes, 20, OutlineProblem::BadModuleHeaderTooLong),
        modules,
    )
    .map(Exposed::Dict)
    .map_err(|not_dict| Problem::OneOf(Box::new(not_list), vec![not_dict]))
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
