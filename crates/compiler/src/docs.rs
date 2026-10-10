//! Package documentation (`Elm/Docs.hs`): an exposed module's doc
//! comment, the `@docs` lines that must list exactly what it exposes,
//! and a comment and an annotation on each of those; then `docs.json`,
//! as `encodeUgly` writes it.
//!
//! 0.19.3 keeps types, values and operators in three namespaces, so it
//! checks and reports the types first, then the values, then the
//! operators, each in its name order; earlier releases go through all
//! the names at once.

use std::collections::BTreeMap;

use nomi_base::diagnostic::render::{self, Styles};
use nomi_base::{Compat, TextRange};
use nomi_syntax::elm::docs::{self as syntax, DocKind, DocName, SyntaxProblem};

use crate::canonical::{self, Associativity, Decl, Def, Type};
use crate::canonicalize::module::{Canonical, Export, Exports};
use crate::canonicalize::name_order;
use crate::hir;
use crate::types::localizer::Localizer;
use crate::types::render::{Context, can_to_doc};

pub mod report;

/// `Docs.Module`. Comments are JSON already, as `Json.fromComment` makes
/// them; types are written out; each kind of entry is in name order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    pub name: String,
    pub comment: String,
    pub unions: Vec<(String, Union)>,
    pub aliases: Vec<(String, Alias)>,
    pub values: Vec<(String, Value)>,
    pub binops: Vec<(String, Binop)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Union {
    pub comment: String,
    pub args: Vec<String>,
    pub cases: Vec<(String, Vec<String>)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias {
    pub comment: String,
    pub args: Vec<String>,
    pub ty: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Value {
    pub comment: String,
    pub ty: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binop {
    pub comment: String,
    pub ty: String,
    pub associativity: Associativity,
    pub precedence: i64,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameProblem {
    Duplicate(String, TextRange, TextRange),
    OnlyInDocs(String, TextRange),
    OnlyInExports(String, TextRange),
}

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
    // The comment between `{-|` and `-}`.
    let start = usize::from(docs.range.start()) + 3;
    let end = usize::from(docs.range.end()) - 2;
    let names = syntax::overview(text, start, end, compat).map_err(Error::SyntaxProblem)?;
    check_names(compat, exports, &names)?;
    check_defs(compat, exports, &text[start..end], module, canonical)
}

/// The space each export is in.
fn space_of(compat: Compat, export: Export) -> Space {
    if compat != Compat::V0_19_3 {
        return Space::Type;
    }
    match export {
        Export::UnionOpen | Export::UnionClosed | Export::Alias => Space::Type,
        Export::Value | Export::Port => Space::Value,
        Export::Binop => Space::Binop,
    }
}

/// `names`, by space, then in each space's name order.
fn ordered<'a, T>(
    compat: Compat,
    names: impl IntoIterator<Item = (Space, &'a String, T)>,
) -> Vec<(&'a String, T)> {
    let order = name_order(compat);
    let mut names: Vec<(Space, &String, T)> = names.into_iter().collect();
    names.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| order(a.1, b.1)));
    names.into_iter().map(|(_, name, t)| (name, t)).collect()
}

/// `checkNames`: every exposed name listed once, and nothing else.
fn check_names(
    compat: Compat,
    exports: &BTreeMap<String, (TextRange, Export)>,
    names: &[DocName],
) -> Result<(), Error> {
    let doc_space = |kind| match (compat, kind) {
        (Compat::V0_19_3, DocKind::Value) => Space::Value,
        (Compat::V0_19_3, DocKind::Binop) => Space::Binop,
        _ => Space::Type,
    };
    // Each listed name's places, in the order written.
    let mut listed: BTreeMap<(Space, &String), Vec<TextRange>> = BTreeMap::new();
    for name in names {
        let key = (doc_space(name.kind), &name.name);
        listed.entry(key).or_default().push(name.range);
    }
    let mut keys: BTreeMap<(Space, &String), ()> = BTreeMap::new();
    for (name, (_, export)) in exports {
        keys.insert((space_of(compat, *export), name), ());
    }
    for key in listed.keys() {
        keys.insert(*key, ());
    }
    let mut problems = Vec::new();
    for (name, space) in ordered(compat, keys.into_keys().map(|(s, n)| (s, n, s))) {
        let export = exports
            .get(name)
            .filter(|(_, e)| space_of(compat, *e) == space);
        match (export, listed.get(&(space, name)).map(Vec::as_slice)) {
            (_, Some([first, second, ..])) => {
                problems.push(NameProblem::Duplicate(name.clone(), *first, *second));
            }
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

/// `checkDefs`: a comment on everything exposed, and an annotation on
/// every value, gathering what the documentation says of each.
fn check_defs(
    compat: Compat,
    exports: &BTreeMap<String, (TextRange, Export)>,
    overview: &str,
    module: &hir::Module,
    canonical: &Canonical,
) -> Result<Module, Error> {
    let types = gather_types(&canonical.decls);
    let comments = comments(module);
    let localizer = Localizer::from_names(compat, []);
    let show = |ty: &Type| type_to_string(compat, &localizer, ty);
    let comment = |name: &str, range: TextRange| {
        (comments.get(name))
            .map(|c| from_comment(c))
            .ok_or_else(|| DefProblem::NoComment(name.to_owned(), range))
    };
    let ty = |name: &str, report: &str| match types.get(name) {
        Some(Ok(ty)) => Ok(show(ty)),
        Some(Err(region)) => Err(DefProblem::NoAnnotation(report.to_owned(), *region)),
        // A port, which no package has.
        None => Ok(canonical
            .annotations
            .get(name)
            .map(|a| show(&a.ty))
            .unwrap_or_default()),
    };
    let mut problems = Vec::new();
    let mut docs = Module {
        name: canonical.home.module.clone(),
        comment: from_comment(overview),
        unions: Vec::new(),
        aliases: Vec::new(),
        values: Vec::new(),
        binops: Vec::new(),
    };
    let entries = ordered(
        compat,
        exports
            .iter()
            .map(|(name, (range, export))| (space_of(compat, *export), name, (*range, *export))),
    );
    for (name, (range, export)) in entries {
        let checked = match export {
            Export::Value | Export::Port => (|| {
                let ty = ty(name, name)?;
                let comment = comment(name, range)?;
                docs.values.push((name.clone(), Value { comment, ty }));
                Ok(())
            })(),
            Export::Binop => (|| {
                let binop = &canonical.binops[name];
                let real = &binop.function;
                let reported = if compat == Compat::V0_19_3 {
                    name
                } else {
                    real
                };
                let ty = ty(real, reported)?;
                let comment = comment(real, range)?;
                docs.binops.push((
                    name.clone(),
                    Binop {
                        comment,
                        ty,
                        associativity: binop.associativity,
                        precedence: binop.precedence,
                    },
                ));
                Ok(())
            })(),
            Export::Alias => comment(name, range).map(|comment| {
                let alias = &canonical.aliases[name];
                docs.aliases.push((
                    name.clone(),
                    Alias {
                        comment,
                        args: alias.vars.clone(),
                        ty: show(&alias.ty),
                    },
                ));
            }),
            Export::UnionOpen | Export::UnionClosed => comment(name, range).map(|comment| {
                let union = &canonical.unions[name];
                let cases = if export == Export::UnionOpen {
                    (union.ctors.iter())
                        .map(|c| (c.name.clone(), c.args.iter().map(&show).collect()))
                        .collect()
                } else {
                    Vec::new()
                };
                docs.unions.push((
                    name.clone(),
                    Union {
                        comment,
                        args: union.vars.clone(),
                        cases,
                    },
                ));
            }),
        };
        if let Err(problem) = checked {
            problems.push(problem);
        }
    }
    if !problems.is_empty() {
        return Err(Error::DefProblems(problems));
    }
    let order = name_order(compat);
    docs.unions.sort_by(|a, b| order(&a.0, &b.0));
    docs.aliases.sort_by(|a, b| order(&a.0, &b.0));
    docs.values.sort_by(|a, b| order(&a.0, &b.0));
    docs.binops.sort_by(|a, b| order(&a.0, &b.0));
    Ok(docs)
}

/// `gatherTypes`: each top-level value's annotation, or where it is
/// defined when it has none.
fn gather_types(decls: &[Decl]) -> BTreeMap<String, Result<Type, TextRange>> {
    let mut types = BTreeMap::new();
    let mut add = |def: &Def| {
        let ty = match def {
            Def::Def { region, .. } => Err(*region),
            Def::TypedDef { params, result, .. } => {
                Ok(params.iter().rev().fold(result.clone(), |ty, (_, arg)| {
                    Type::Lambda(Box::new(arg.clone()), Box::new(ty))
                }))
            }
        };
        types.insert(def.name().to_owned(), ty);
    };
    for decl in decls {
        match decl {
            Decl::Declare(def) => add(def),
            Decl::DeclareRec(def, defs) => {
                add(def);
                defs.iter().for_each(&mut add);
            }
        }
    }
    types
}

/// Each declaration's doc comment, between its `{-|` and `-}`, by name
/// (`getComments`).
fn comments(module: &hir::Module) -> BTreeMap<&str, &str> {
    fn inner(docs: &Option<hir::Spanned>) -> Option<&str> {
        docs.as_ref()
            .map(|d| &d.text[3..d.text.len().saturating_sub(2).max(3)])
    }
    let mut comments = BTreeMap::new();
    for &id in &module.values {
        if let hir::Def::Define { name, docs, .. } = &module.defs[id]
            && let Some(comment) = inner(docs)
        {
            comments.insert(name.text.as_str(), comment);
        }
    }
    let rest = (module.unions.iter().map(|u| (&u.name, &u.docs)))
        .chain(module.aliases.iter().map(|a| (&a.name, &a.docs)))
        .chain(module.ports.iter().map(|p| (&p.name, &p.docs)));
    for (name, docs) in rest {
        if let Some(comment) = inner(docs) {
            comments.insert(name.text.as_str(), comment);
        }
    }
    comments
}

/// `Json.fromComment`: newlines, quotes and backslashes escaped, carriage
/// returns dropped, everything else as written.
fn from_comment(comment: &str) -> String {
    let mut out = String::with_capacity(comment.len());
    for c in comment.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// `Type.encode`'s text: `toDoc L.empty`, every type named by its
/// module, laid out on one line (`D.toLine`).
fn type_to_string(compat: Compat, localizer: &Localizer, ty: &canonical::Type) -> String {
    let doc = can_to_doc(localizer, Context::None, ty);
    render::render(&doc, compat, usize::MAX / 4, Styles::Plain)
}

/// `E.chars`: a JSON string, with `\r`, `\n`, `"` and `\` escaped.
fn chars(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `encode` through `encodeUgly`: `docs.json` for `modules`, in their
/// names' order.
pub fn encode(compat: Compat, modules: &[Module]) -> String {
    let order = name_order(compat);
    let mut modules: Vec<&Module> = modules.iter().collect();
    modules.sort_by(|a, b| order(&a.name, &b.name));
    let quoted = |s: &str| format!("\"{s}\"");
    let list = |items: Vec<String>| format!("[{}]", items.join(","));
    let names = |names: &[String]| list(names.iter().map(|n| quoted(n)).collect());
    let entries: Vec<String> = modules
        .into_iter()
        .map(|m| {
            let unions = (m.unions.iter())
                .map(|(name, u)| {
                    let cases = (u.cases.iter())
                        .map(|(tag, args)| {
                            format!(
                                "[{},{}]",
                                quoted(tag),
                                list(args.iter().map(|a| chars(a)).collect())
                            )
                        })
                        .collect();
                    format!(
                        "{{\"name\":{},\"comment\":{},\"args\":{},\"cases\":{}}}",
                        quoted(name),
                        quoted(&u.comment),
                        names(&u.args),
                        list(cases)
                    )
                })
                .collect();
            let aliases = (m.aliases.iter())
                .map(|(name, a)| {
                    format!(
                        "{{\"name\":{},\"comment\":{},\"args\":{},\"type\":{}}}",
                        quoted(name),
                        quoted(&a.comment),
                        names(&a.args),
                        chars(&a.ty)
                    )
                })
                .collect();
            let values = (m.values.iter())
                .map(|(name, v)| {
                    format!(
                        "{{\"name\":{},\"comment\":{},\"type\":{}}}",
                        quoted(name),
                        quoted(&v.comment),
                        chars(&v.ty)
                    )
                })
                .collect();
            let binops = (m.binops.iter())
                .map(|(name, b)| {
                    let associativity = match b.associativity {
                        Associativity::Left => "left",
                        Associativity::Non => "non",
                        Associativity::Right => "right",
                    };
                    format!(
                        "{{\"name\":{},\"comment\":{},\"type\":{},\"associativity\":{},\"precedence\":{}}}",
                        quoted(name),
                        quoted(&b.comment),
                        chars(&b.ty),
                        quoted(associativity),
                        b.precedence
                    )
                })
                .collect();
            format!(
                "{{\"name\":{},\"comment\":{},\"unions\":{},\"aliases\":{},\"values\":{},\"binops\":{}}}",
                quoted(&m.name),
                quoted(&m.comment),
                list(unions),
                list(aliases),
                list(values),
                list(binops)
            )
        })
        .collect();
    list(entries)
}
