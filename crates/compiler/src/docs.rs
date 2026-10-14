//! Package documentation (`Elm/Docs.hs`): an exposed module's doc
//! comment, the `@docs` lines that must list exactly what it exposes,
//! and a comment and an annotation on each of those; then `docs.json`,
//! as `encodeUgly` writes it.
//!
//! 0.19.3 keeps types, values and operators in three namespaces, so it
//! checks and reports the types first, then the values, then the
//! operators, each in its name order; earlier releases go through all
//! the names at once.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write};

use nomi_base::diagnostic::json;
use nomi_base::diagnostic::render::{self, Styles};
use nomi_base::{Compat, TextRange};
use nomi_syntax::elm::docs::{self as syntax, DocKind, DocName, SyntaxProblem};

use crate::canonical::{Decl, Def, Type};
use crate::canonicalize::module::{Canonical, Export, Exports};
use crate::hir::{self, Fixity};
use crate::types::localizer::Localizer;
use crate::types::render::{Context, can_to_doc};

pub mod report;

/// `Docs.Module`. Comments are as written, between their `{-|` and `-}`;
/// types are written out; each kind of entry is in name order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    pub name: String,
    pub comment: String,
    pub unions: Vec<(String, Union)>,
    pub aliases: Vec<(String, Alias)>,
    pub values: Vec<(String, Value)>,
    pub binops: Vec<(String, Binop)>,
}

/// `Docs.Union`: its parameters and, when exposed, its constructors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Union {
    pub comment: String,
    pub args: Vec<String>,
    pub cases: Vec<(String, Vec<String>)>,
}

/// `Docs.Alias`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias {
    pub comment: String,
    pub args: Vec<String>,
    pub ty: String,
}

/// `Docs.Value`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Value {
    pub comment: String,
    pub ty: String,
}

/// `Docs.Binop`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binop {
    pub comment: String,
    pub ty: String,
    pub fixity: Fixity,
}

/// `Reporting.Error.Docs.Error`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    NoDocs(TextRange),
    ImplicitExposing(TextRange),
    SyntaxProblem(SyntaxProblem),
    NameProblems(Vec<NameProblem>),
    DefProblems(Vec<DefProblem>),
}

/// How the `@docs` lines and the exports disagree about a name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameProblem {
    Duplicate {
        name: String,
        first: TextRange,
        second: TextRange,
    },
    OnlyInDocs(String, TextRange),
    OnlyInExports(String, TextRange),
}

/// What an exposed declaration lacks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefProblem {
    NoComment(String, TextRange),
    NoAnnotation(String, TextRange),
}

/// The namespaces 0.19.3 checks one after another; earlier releases have
/// one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Space {
    Type,
    Value,
    Binop,
}

impl Space {
    /// The space `export` is in.
    fn of(compat: Compat, export: Export) -> Space {
        let space = match export {
            Export::UnionOpen | Export::UnionClosed | Export::Alias => Space::Type,
            Export::Value | Export::Port => Space::Value,
            Export::Binop => Space::Binop,
        };
        space.under(compat)
    }

    /// The space a `@docs` name of `kind` is in.
    fn of_doc(compat: Compat, kind: DocKind) -> Space {
        let space = match kind {
            DocKind::Type => Space::Type,
            DocKind::Value => Space::Value,
            DocKind::Binop => Space::Binop,
        };
        space.under(compat)
    }

    /// The space in `compat`, which before 0.19.3 is the one.
    fn under(self, compat: Compat) -> Space {
        match compat {
            Compat::V0_19_1 | Compat::V0_19_2 => Space::Type,
            Compat::V0_19_3 => self,
        }
    }
}

/// `fromModule`: the documentation of `module`, whose source is `text`
/// and which canonicalized as `canonical`.
pub fn from_module(
    compat: Compat,
    text: &str,
    module: &hir::Module,
    canonical: &Canonical,
) -> Result<Module, Error> {
    let exports = match &canonical.exports {
        Exports::Everything(range) => return Err(Error::ImplicitExposing(*range)),
        Exports::Explicit(exports) => exports,
    };
    let Some(docs) = &module.docs else {
        return Err(Error::NoDocs(module.docs_gap));
    };
    let overview = body(&docs.text);
    let start = usize::from(docs.range.start()) + "{-|".len();
    let names = syntax::overview(text, start, start + overview.len(), compat)
        .map_err(Error::SyntaxProblem)?;
    check_names(compat, exports, &names)?;
    check_defs(compat, exports, overview, module, canonical)
}

/// A doc comment's text between its `{-|` and `-}`.
fn body(comment: &str) -> &str {
    let body = comment.strip_prefix("{-|").unwrap_or(comment);
    body.strip_suffix("-}").unwrap_or(body)
}

/// `names`, by space, then in each space's name order.
fn ordered<'a, T>(
    compat: Compat,
    names: impl IntoIterator<Item = (Space, &'a String, T)>,
) -> Vec<(&'a String, T)> {
    let mut names: Vec<(Space, &String, T)> = names.into_iter().collect();
    names.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| compat.compare_names(a.1, b.1)));
    names.into_iter().map(|(_, name, t)| (name, t)).collect()
}

/// `checkNames`: every exposed name listed once, and nothing else.
fn check_names(
    compat: Compat,
    exports: &BTreeMap<String, (TextRange, Export)>,
    names: &[DocName],
) -> Result<(), Error> {
    // Each listed name's places, in the order written.
    let mut listed: BTreeMap<(Space, &String), Vec<TextRange>> = BTreeMap::new();
    for name in names {
        let key = (Space::of_doc(compat, name.kind), &name.name);
        listed.entry(key).or_default().push(name.range);
    }
    let mut keys: BTreeSet<(Space, &String)> = (exports.iter())
        .map(|(name, (_, export))| (Space::of(compat, *export), name))
        .collect();
    keys.extend(listed.keys());
    let mut problems = Vec::new();
    for (name, space) in ordered(compat, keys.into_iter().map(|(s, n)| (s, n, s))) {
        let export = exports
            .get(name)
            .filter(|(_, e)| Space::of(compat, *e) == space);
        match (export, listed.get(&(space, name)).map(Vec::as_slice)) {
            (_, Some([first, second, ..])) => problems.push(NameProblem::Duplicate {
                name: name.clone(),
                first: *first,
                second: *second,
            }),
            (None, Some([only])) => problems.push(NameProblem::OnlyInDocs(name.clone(), *only)),
            (Some((range, _)), None) => {
                problems.push(NameProblem::OnlyInExports(name.clone(), *range));
            }
            _ => {}
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(Error::NameProblems(problems))
    }
}

/// What `checkDefs` reads of a module: each declaration's comment and
/// each value's type.
struct Defs<'a> {
    compat: Compat,
    canonical: &'a Canonical,
    comments: BTreeMap<&'a str, &'a str>,
    defs: BTreeMap<&'a str, &'a Def>,
    localizer: Localizer,
}

impl Defs<'_> {
    fn show(&self, ty: &Type) -> String {
        type_to_string(self.compat, &self.localizer, ty)
    }

    fn comment(&self, name: &str, range: TextRange) -> Result<String, DefProblem> {
        (self.comments.get(name))
            .map(|c| (*c).to_owned())
            .ok_or_else(|| DefProblem::NoComment(name.to_owned(), range))
    }

    /// The type of the value `name`, reported as `report` when it has no
    /// annotation.
    fn ty(&self, name: &str, report: &str) -> Result<String, DefProblem> {
        match self.defs.get(name) {
            Some(Def::TypedDef { params, result, .. }) => {
                let args = params.iter().map(|(_, arg)| arg.clone());
                Ok(self.show(&Type::function(args, result.clone())))
            }
            Some(Def::Def { region, .. }) => {
                Err(DefProblem::NoAnnotation(report.to_owned(), *region))
            }
            // A port, which no package has.
            None => Ok((self.canonical.annotations.get(name))
                .map(|a| self.show(&a.ty))
                .unwrap_or_default()),
        }
    }

    fn value(&self, name: &str, range: TextRange) -> Result<Value, DefProblem> {
        let ty = self.ty(name, name)?;
        let comment = self.comment(name, range)?;
        Ok(Value { comment, ty })
    }

    fn binop(&self, name: &str, range: TextRange) -> Result<Binop, DefProblem> {
        let binop = &self.canonical.binops[name];
        let real = &binop.function;
        let reported = if self.compat == Compat::V0_19_3 {
            name
        } else {
            real
        };
        let ty = self.ty(real, reported)?;
        let comment = self.comment(real, range)?;
        Ok(Binop {
            comment,
            ty,
            fixity: binop.fixity,
        })
    }

    fn alias(&self, name: &str, range: TextRange) -> Result<Alias, DefProblem> {
        let comment = self.comment(name, range)?;
        let alias = &self.canonical.aliases[name];
        Ok(Alias {
            comment,
            args: alias.vars.clone(),
            ty: self.show(&alias.ty),
        })
    }

    /// A union, its constructors listed where `export` exposes them.
    fn union(&self, name: &str, range: TextRange, export: Export) -> Result<Union, DefProblem> {
        let comment = self.comment(name, range)?;
        let union = &self.canonical.unions[name];
        let cases = if export == Export::UnionOpen {
            (union.ctors.iter())
                .map(|c| {
                    (
                        c.name.clone(),
                        c.args.iter().map(|t| self.show(t)).collect(),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(Union {
            comment,
            args: union.vars.clone(),
            cases,
        })
    }
}

/// `checkDefs`: a comment on everything exposed, and an annotation on
/// every value, gathering what the documentation says of each.
fn check_defs(
    compat: Compat,
    exports: &BTreeMap<String, (TextRange, Export)>,
    overview: &str,
    module: &hir::Module,
    canonical: &Canonical,
) -> Result<Module, Error> {
    let defs = Defs {
        compat,
        canonical,
        comments: comments(module),
        defs: top_level(&canonical.decls),
        localizer: Localizer::from_names(compat, []),
    };
    let mut problems = Vec::new();
    let mut docs = Module {
        name: canonical.home.module.to_string(),
        comment: overview.to_owned(),
        unions: Vec::new(),
        aliases: Vec::new(),
        values: Vec::new(),
        binops: Vec::new(),
    };
    let entries = ordered(
        compat,
        (exports.iter())
            .map(|(name, (range, export))| (Space::of(compat, *export), name, (*range, *export))),
    );
    // Each kind's entries come out of `ordered` in its name order, since
    // all of a kind share a space.
    for (name, (range, export)) in entries {
        let checked = match export {
            Export::Value | Export::Port => {
                (defs.value(name, range)).map(|v| docs.values.push((name.clone(), v)))
            }
            Export::Binop => (defs.binop(name, range)).map(|b| docs.binops.push((name.clone(), b))),
            Export::Alias => {
                (defs.alias(name, range)).map(|a| docs.aliases.push((name.clone(), a)))
            }
            Export::UnionOpen | Export::UnionClosed => {
                (defs.union(name, range, export)).map(|u| docs.unions.push((name.clone(), u)))
            }
        };
        if let Err(problem) = checked {
            problems.push(problem);
        }
    }
    if problems.is_empty() {
        Ok(docs)
    } else {
        Err(Error::DefProblems(problems))
    }
}

/// Each top-level value's definition, by name, for `gatherTypes`, whose
/// types are made only for the values documented.
fn top_level(decls: &[Decl]) -> BTreeMap<&str, &Def> {
    let mut defs = BTreeMap::new();
    for decl in decls {
        let (def, rest) = match decl {
            Decl::Declare(def) => (def, &[][..]),
            Decl::DeclareRec(def, rest) => (def, &rest[..]),
        };
        for def in std::iter::once(def).chain(rest) {
            defs.insert(def.name(), def);
        }
    }
    defs
}

/// Each declaration's doc comment, between its `{-|` and `-}`, by name
/// (`getComments`).
fn comments(module: &hir::Module) -> BTreeMap<&str, &str> {
    let mut comments = BTreeMap::new();
    for &id in &module.values {
        if let hir::Def::Define { name, docs, .. } = &module.defs[id]
            && let Some(docs) = docs
        {
            comments.insert(name.text.as_str(), body(&docs.text));
        }
    }
    let rest = (module.unions.iter().map(|u| (&u.name, &u.docs)))
        .chain(module.aliases.iter().map(|a| (&a.name, &a.docs)))
        .chain(module.ports.iter().map(|p| (&p.name, &p.docs)));
    for (name, docs) in rest {
        if let Some(docs) = docs {
            comments.insert(name.text.as_str(), body(&docs.text));
        }
    }
    comments
}

/// What JSON escaping does with a carriage return.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Returns {
    /// As `E.chars` does.
    Escape,
    /// As `Json.fromComment` does.
    Drop,
}

/// Text as a JSON string: newlines, quotes and backslashes escaped,
/// carriage returns as `Returns` says, and everything else as written.
struct JsonString<'t>(&'t str, Returns);

impl fmt::Display for JsonString<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let JsonString(text, returns) = *self;
        f.write_char('"')?;
        let mut done = 0;
        for (i, byte) in text.bytes().enumerate() {
            if let Some(escaped) = json::escape(byte) {
                // What `escape` escapes is ASCII, so `i` starts a character.
                f.write_str(&text[done..i])?;
                if byte != b'\r' || returns == Returns::Escape {
                    f.write_str(escaped)?;
                }
                done = i + 1;
            }
        }
        f.write_str(&text[done..])?;
        f.write_char('"')
    }
}

/// `Json.fromComment`.
fn comment(text: &str) -> JsonString<'_> {
    JsonString(text, Returns::Drop)
}

/// `E.chars`: a JSON string.
fn chars(text: &str) -> JsonString<'_> {
    JsonString(text, Returns::Escape)
}

/// `Type.encode`'s text: `toDoc L.empty`, every type named by its
/// module, laid out on one line (`D.toLine`).
fn type_to_string(compat: Compat, localizer: &Localizer, ty: &Type) -> String {
    let doc = can_to_doc(localizer, Context::None, ty);
    render::render(&doc, compat, usize::MAX / 4, Styles::Plain)
}

/// `encode` through `encodeUgly`: `docs.json` for `modules`, in their
/// names' order.
pub fn encode(compat: Compat, modules: &[Module]) -> String {
    let mut modules: Vec<&Module> = modules.iter().collect();
    modules.sort_by(|a, b| compat.compare_names(&a.name, &b.name));
    let mut out = String::new();
    // Writing to a `String` can't fail.
    let _ = list(&mut out, modules, encode_module);
    out
}

/// A JSON array of `items`, each written by `item`.
fn list<T>(
    out: &mut String,
    items: impl IntoIterator<Item = T>,
    mut item: impl FnMut(&mut String, T) -> fmt::Result,
) -> fmt::Result {
    out.push('[');
    for (i, x) in items.into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        item(out, x)?;
    }
    out.push(']');
    Ok(())
}

/// Names, each quoted as it is.
fn names(out: &mut String, names: &[String]) -> fmt::Result {
    list(out, names, |out, name| write!(out, "\"{name}\""))
}

/// `encodeModule`.
fn encode_module(out: &mut String, m: &Module) -> fmt::Result {
    let name = &m.name;
    write!(
        out,
        "{{\"name\":\"{name}\",\"comment\":{},\"unions\":",
        comment(&m.comment)
    )?;
    list(out, &m.unions, |out, (name, u)| {
        write!(
            out,
            "{{\"name\":\"{name}\",\"comment\":{},\"args\":",
            comment(&u.comment)
        )?;
        names(out, &u.args)?;
        out.push_str(",\"cases\":");
        list(out, &u.cases, |out, (tag, args)| {
            write!(out, "[\"{tag}\",")?;
            list(out, args, |out, arg| write!(out, "{}", chars(arg)))?;
            out.push(']');
            Ok(())
        })?;
        out.push('}');
        Ok(())
    })?;
    out.push_str(",\"aliases\":");
    list(out, &m.aliases, |out, (name, a)| {
        write!(
            out,
            "{{\"name\":\"{name}\",\"comment\":{},\"args\":",
            comment(&a.comment)
        )?;
        names(out, &a.args)?;
        write!(out, ",\"type\":{}}}", chars(&a.ty))
    })?;
    out.push_str(",\"values\":");
    list(out, &m.values, |out, (name, v)| {
        let (comment, ty) = (comment(&v.comment), chars(&v.ty));
        write!(
            out,
            "{{\"name\":\"{name}\",\"comment\":{comment},\"type\":{ty}}}"
        )
    })?;
    out.push_str(",\"binops\":");
    list(out, &m.binops, |out, (name, b)| {
        let associativity = b.fixity.associativity.json_name();
        let (comment, ty, precedence) = (comment(&b.comment), chars(&b.ty), b.fixity.precedence);
        write!(
            out,
            "{{\"name\":\"{name}\",\"comment\":{comment},\"type\":{ty},\"associativity\":\"{associativity}\",\"precedence\":{precedence}}}"
        )
    })?;
    out.push('}');
    Ok(())
}
