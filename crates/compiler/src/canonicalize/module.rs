//! A module canonicalized (`Canonicalize/Module.hs`): the environment of
//! its imports, its own declarations, its values, and its exports, each
//! stopping what follows when it fails; and the interface it offers
//! (`Elm/Interface.hs`, `fromModule`).

use std::collections::BTreeMap;

use nomi_base::{Compat, TextRange};

use super::cycles::{self, TopNode};
use super::dups::Dict;
use super::effects;
use super::env::Env;
use super::error::{Error, NameKind, R, all, throw};
use super::expr;
use super::foreign::{initial_env, with_defaults};
use super::local::{self, Aliases, Unions};
use super::pattern;
use super::references::{Kind, Recorder, Reference};
use crate::canonical::{Annotation, Decl, Effects, Home, Pattern, Type};
use crate::hir::{self, Exposed, Exposing, Fixity, Spanned};
use crate::interface::{self, Interface, Interfaces};

/// What a module exposes (`Can.Exports`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exports {
    /// `(..)`, and where it is.
    Everything(TextRange),
    /// Each name exposed, where the list names it, and what it is.
    Explicit(BTreeMap<String, (TextRange, Export)>),
}

/// `Can.Export`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Export {
    Value,
    UnionOpen,
    UnionClosed,
    Alias,
    Binop,
    Port,
}

/// `Can.Binop_`: an operator's grouping and the function it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binop {
    pub fixity: Fixity,
    pub function: String,
}

/// A module canonicalized: what its interface, inference and the
/// optimizer read.
#[derive(Clone, Debug)]
pub struct Canonical {
    pub home: Home,
    pub exports: Exports,
    pub unions: Unions,
    pub aliases: Aliases,
    pub binops: BTreeMap<String, Binop>,
    /// Each top-level value's annotation, where it has one.
    pub annotations: BTreeMap<String, Annotation>,
    /// Every top-level value's name, a port module's ports included.
    pub values: Vec<String>,
    /// The values, each before those that use it (`Can.Decls`).
    pub decls: Vec<Decl>,
    pub effects: Effects,
}

/// `canonicalize`: `module`, in `package`, with `interfaces` holding the
/// interface of every module it imports by name.
pub fn canonicalize(
    compat: Compat,
    package: &str,
    interfaces: &dyn Interfaces,
    module: &hir::Module,
) -> R<Canonical> {
    canonicalize_with_references(compat, package, interfaces, module).0
}

/// [`canonicalize`], and what each name resolved to, as far as it got.
pub fn canonicalize_with_references(
    compat: Compat,
    package: &str,
    interfaces: &dyn Interfaces,
    module: &hir::Module,
) -> (R<Canonical>, Vec<Reference>) {
    let references = Recorder::default();
    let canonical = canonicalize_into(compat, package, interfaces, module, &references);
    (canonical, references.take())
}

fn canonicalize_into(
    compat: Compat,
    package: &str,
    interfaces: &dyn Interfaces,
    module: &hir::Module,
    references: &Recorder,
) -> R<Canonical> {
    let home = Home {
        package: package.into(),
        module: module.name().into(),
    };
    let binops = (module.binops.iter())
        .map(|b| {
            let binop = Binop {
                fixity: b.fixity,
                function: b.function.text.clone(),
            };
            (b.op.text.clone(), binop)
        })
        .collect();
    let imports = with_defaults(package, &module.imports);
    // The module's own exposing list names its declarations.
    if let Exposing::Explicit(items) = &module.exports {
        for item in items {
            let kind = match item {
                Exposed::Lower(_) => Kind::Value,
                Exposed::Upper(..) => Kind::Type,
                Exposed::Operator(_) => Kind::Binop,
            };
            let name = item.name();
            references.global(name.range, &home, &name.text, kind);
        }
    }
    let env = initial_env(compat, &home, interfaces, &imports, references)?;
    let (env, unions, aliases) = local::add(module, env)?;
    // `canonicalizeValues`, in upstream's reverse order.
    let nodes = all(module
        .values
        .iter()
        .rev()
        .map(|&id| value(&env, module, id)))?;
    let mut annotations = BTreeMap::new();
    let top = (nodes.into_iter().flatten())
        .map(|(node, annotation)| {
            if let Some(annotation) = annotation {
                annotations.insert(node.def.name().to_owned(), annotation);
            }
            node
        })
        .collect();
    let decls = cycles::top_level(compat, top)?;
    let effects = effects::canonicalize(&env, module, &unions)?;
    let mut values: Vec<String> = module.value_names().map(|n| n.text.clone()).collect();
    let declared = Declared {
        compat,
        port_module: matches!(module.kind, hir::ModuleKind::Port),
        values: &values,
        ports: &module.ports,
        unions: &unions,
        aliases: &aliases,
        binops: &binops,
    };
    let exports = exports(&declared, module)?;
    // Ports are values too, which importers may use.
    if let Effects::Ports(ports) = &effects {
        for (name, port) in ports {
            values.push(name.clone());
            let annotation = Annotation {
                free: port.free.clone(),
                ty: port.func.clone().into(),
            };
            annotations.insert(name.clone(), annotation);
        }
    }
    Ok(Canonical {
        home,
        exports,
        unions,
        aliases,
        binops,
        annotations,
        values,
        decls,
        effects,
    })
}

/// `toNodeOne`: a value canonicalized, with its annotation, which the
/// interface reads; none for a destructuring, which elm rejects at the
/// top level.
fn value(
    env: &Env<'_>,
    module: &hir::Module,
    id: hir::DefId,
) -> R<Option<(TopNode, Option<Annotation>)>> {
    let hir::Def::Define {
        name,
        params,
        body,
        annotation,
        ..
    } = &module.defs[id]
    else {
        return Ok(None);
    };
    let defined = expr::definition(env, module, name, params, *body, *annotation)?;
    let node = TopNode {
        def: defined.def,
        free: defined.free,
    };
    Ok(Some((node, defined.annotation)))
}

/// `gatherTypedArgs`: each parameter against the argument its annotation
/// gives it, stopping at the first that fails, or where the annotation
/// runs out of arguments.
pub(super) fn typed_args(
    env: &Env<'_>,
    module: &hir::Module,
    name: &str,
    params: &[hir::PatId],
    ty: &Type,
    bindings: &mut pattern::Bindings,
) -> R<(Vec<(Pattern, Type)>, Type)> {
    let mut ty = ty.clone();
    let mut typed = Vec::with_capacity(params.len());
    for (index, &param) in params.iter().enumerate() {
        if let Type::Lambda(arg, result) = ty.iterated_dealias() {
            let pattern = pattern::canonicalize(env, module, param, bindings)?;
            typed.push((pattern, *arg));
            ty = *result;
        } else {
            let range = |id| module.source_map.pattern_range(id);
            let first = range(param);
            let last = range(*params.last().expect("a parameter"));
            return throw(Error::AnnotationTooShort {
                range: TextRange::new(first.start(), last.end()),
                name: name.to_owned(),
                index,
                leftover: params.len() - index,
            });
        }
    }
    Ok((typed, ty))
}

/// What a module declares, which its exposing list names.
struct Declared<'a> {
    compat: Compat,
    port_module: bool,
    values: &'a [String],
    ports: &'a [hir::Port],
    unions: &'a Unions,
    aliases: &'a Aliases,
    binops: &'a BTreeMap<String, Binop>,
}

impl Declared<'_> {
    fn value(&self, value: &Spanned) -> R<Export> {
        if self.values.contains(&value.text) {
            return Ok(Export::Value);
        }
        let ports = self.ports.iter().map(|p| &p.name.text);
        if self.port_module && ports.clone().any(|p| *p == value.text) {
            return Ok(Export::Port);
        }
        // `ports ++ Map.keys values`, each in name order.
        let mut possible = if self.port_module {
            self.compat.sorted_names(ports.cloned())
        } else {
            Vec::new()
        };
        possible.extend(self.compat.sorted_names(self.values.iter().cloned()));
        throw(Error::ExportNotFound {
            range: value.range,
            kind: NameKind::Value,
            name: value.text.clone(),
            possible,
        })
    }

    fn operator(&self, op: &Spanned) -> R<Export> {
        if self.binops.contains_key(&op.text) {
            return Ok(Export::Binop);
        }
        throw(Error::ExportNotFound {
            range: op.range,
            kind: NameKind::Operator,
            name: op.text.clone(),
            possible: self.compat.sorted_names(self.binops.keys().cloned()),
        })
    }

    /// A type, with `(..)`'s range when it is open.
    fn ty(&self, name: &Spanned, open: Option<TextRange>) -> R<Export> {
        let union = self.unions.contains_key(&name.text);
        let alias = self.aliases.contains_key(&name.text);
        match open {
            Some(_) if union => Ok(Export::UnionOpen),
            Some(dots) if alias => throw(Error::ExportOpenAlias {
                range: dots,
                name: name.text.clone(),
            }),
            None if union => Ok(Export::UnionClosed),
            None if alias => Ok(Export::Alias),
            _ => {
                let unions = self.compat.sorted_names(self.unions.keys());
                let aliases = self.compat.sorted_names(self.aliases.keys());
                throw(Error::ExportNotFound {
                    range: name.range,
                    kind: NameKind::Type,
                    name: name.text.clone(),
                    possible: unions.into_iter().chain(aliases).cloned().collect(),
                })
            }
        }
    }
}

/// `canonicalizeExports`.
fn exports(declared: &Declared<'_>, module: &hir::Module) -> R<Exports> {
    let Exposing::Explicit(items) = &module.exports else {
        return Ok(Exports::Everything(module.exports_range));
    };
    let kind = |item: &Exposed| match item {
        Exposed::Lower(_) => NameKind::Value,
        Exposed::Upper(..) => NameKind::Type,
        Exposed::Operator(_) => NameKind::Operator,
    };
    let infos = all(items.iter().map(|item| {
        let export = match item {
            Exposed::Lower(name) => declared.value(name),
            Exposed::Operator(op) => declared.operator(op),
            Exposed::Upper(name, open) => declared.ty(name, *open),
        }?;
        let name = item.name();
        Ok(Dict::one(&name.text, name.range, (name.range, export)))
    }))?;
    let exports = Dict::unions(infos).detect(declared.compat, |name, first, second| {
        // A name's spelling says which kind it is.
        let item = items.iter().find(|i| i.name().text == name);
        Error::ExportDuplicate {
            kind: item.map_or(NameKind::Value, kind),
            name: name.to_owned(),
            first,
            second,
        }
    })?;
    Ok(Exports::Explicit(exports))
}

/// The type variable standing for the type of a value with neither an
/// inferred type nor a written one, as when navigation builds interfaces
/// without inferring: its name is all an importer needs to canonicalize.
const UNKNOWN: &str = "?";

/// How far a module's exports reach a declaration.
enum Exposure {
    /// `exposing (..)`.
    Everything,
    Listed(Export),
    Hidden,
}

/// `fromModule`: what the module offers importers.
pub fn interface(canonical: &Canonical, inferred: &BTreeMap<String, Annotation>) -> Interface {
    // What inference found, else what the source wrote: navigation
    // builds interfaces without inferring.
    let annotation = |name: &str| {
        (inferred.get(name))
            .or_else(|| canonical.annotations.get(name))
            .cloned()
            .unwrap_or_else(|| Annotation {
                free: Vec::new(),
                ty: Type::Var(UNKNOWN.to_owned()).into(),
            })
    };
    let exposure = |name: &str| match &canonical.exports {
        Exports::Everything(_) => Exposure::Everything,
        Exports::Explicit(exports) => {
            (exports.get(name)).map_or(Exposure::Hidden, |(_, export)| Exposure::Listed(*export))
        }
    };
    let exposed = |name: &str| !matches!(exposure(name), Exposure::Hidden);
    let values = (canonical.values.iter())
        .filter(|name| exposed(name))
        .map(|name| (name.clone(), annotation(name)))
        .collect();
    let unions = (canonical.unions.iter())
        .map(|(name, union)| {
            let union = union.clone();
            let union = match exposure(name) {
                Exposure::Everything | Exposure::Listed(Export::UnionOpen) => {
                    interface::Union::Open(union)
                }
                Exposure::Listed(_) => interface::Union::Closed(union),
                Exposure::Hidden => interface::Union::Private(union),
            };
            (name.clone(), union)
        })
        .collect();
    let aliases = (canonical.aliases.iter())
        .map(|(name, alias)| {
            let alias = if exposed(name) {
                interface::Alias::Public(alias.clone())
            } else {
                interface::Alias::Private(alias.clone())
            };
            (name.clone(), alias)
        })
        .collect();
    let binops = (canonical.binops.iter())
        .filter(|(op, _)| exposed(op))
        .map(|(op, b)| {
            let binop = interface::Binop {
                function: b.function.clone(),
                annotation: annotation(&b.function),
                fixity: b.fixity,
            };
            (op.clone(), binop)
        })
        .collect();
    Interface {
        home: canonical.home.package.to_string(),
        values,
        unions,
        aliases,
        binops,
    }
}
