//! A module canonicalized (`Canonicalize/Module.hs`): the environment of
//! its imports, its own declarations, its values, and its exports, each
//! stopping what follows when it fails; and the interface it offers
//! (`Elm/Interface.hs`, `fromModule`).

use std::collections::{BTreeMap, HashMap};

use nomi_base::{Compat, TextRange};

use super::cycles;
use super::dups::Dict;
use super::effects;
use super::env::Env;
use super::error::{Error, NameKind, R, all, throw};
use super::expr::{self, FreeLocals};
use super::foreign::{initial_env, with_defaults};
use super::local::{self, Aliases, Unions};
use super::pattern;
use super::references::{Kind, Recorder, Reference};
use super::types;
use crate::canonical::{
    AliasType, Annotation, Associativity, Decl as CDecl, Def as CDef, Effects, Home, Pattern, Type,
};
use crate::hir::{self, Def, Exposed, Exposing};
use crate::interface::{self, Interface};

/// What a module exposes (`Can.Exports`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exports {
    Everything,
    Explicit(BTreeMap<String, Export>),
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
    pub associativity: Associativity,
    pub precedence: i64,
    pub function: String,
}

/// A module canonicalized, as far as nomi canonicalizes so far.
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
    pub decls: Vec<CDecl>,
    pub effects: Effects,
}

/// The module's name, `Main` without a header.
pub fn name(module: &hir::Module) -> &str {
    module.name.as_ref().map_or("Main", |n| n.text.as_str())
}

/// `canonicalize`: `module`, in `package`, with `interfaces` holding the
/// interface of every module it imports by name.
pub fn canonicalize(
    compat: Compat,
    package: &str,
    interfaces: &HashMap<String, Interface>,
    module: &hir::Module,
) -> R<Canonical> {
    canonicalize_with_references(compat, package, interfaces, module).0
}

/// [`canonicalize`], and what each name resolved to, as far as it got.
pub fn canonicalize_with_references(
    compat: Compat,
    package: &str,
    interfaces: &HashMap<String, Interface>,
    module: &hir::Module,
) -> (R<Canonical>, Vec<Reference>) {
    let references = Recorder::default();
    let canonical = canonicalize_into(compat, package, interfaces, module, &references);
    (canonical, references.take())
}

fn canonicalize_into(
    compat: Compat,
    package: &str,
    interfaces: &HashMap<String, Interface>,
    module: &hir::Module,
    references: &Recorder,
) -> R<Canonical> {
    let home = Home {
        package: package.to_owned(),
        module: name(module).to_owned(),
    };
    let binops = (module.binops.iter())
        .map(|b| {
            let binop = Binop {
                associativity: b.associativity,
                precedence: b.precedence,
                function: b.function.text.clone(),
            };
            (b.op.text.clone(), binop)
        })
        .collect();
    let imports = with_defaults(package, &module.imports);
    // The module's own exposing list names its declarations.
    if let hir::Exposing::Explicit(items) = &module.exports {
        for item in items {
            let (name, kind) = match item {
                hir::Exposed::Lower(n) => (n, Kind::Value),
                hir::Exposed::Upper(n, _) => (n, Kind::Type),
                hir::Exposed::Operator(n) => (n, Kind::Binop),
            };
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
    let nodes: Vec<ValueNode> = nodes.into_iter().flatten().collect();
    let annotations = (nodes.iter())
        .filter_map(|n| Some((n.name.clone(), n.annotation.clone()?)))
        .collect();
    let top = (nodes.into_iter())
        .map(|n| cycles::TopNode {
            def: n.def,
            has_params: n.has_params,
            free: n.free,
        })
        .collect();
    let decls = cycles::top_level(compat, top)?;
    let effects = effects::canonicalize(&env, module, &unions)?;
    // Ports are values too, which importers may use.
    let ports_annotated = (module.ports.iter())
        .map(|p| {
            Ok((
                p.name.text.clone(),
                types::to_annotation(&env, module, p.ty)?,
            ))
        })
        .collect::<R<Vec<_>>>()?;
    let values: Vec<String> = (module.values.iter())
        .filter_map(|&id| match &module.defs[id] {
            Def::Define { name, .. } => Some(name.text.clone()),
            Def::Destruct { .. } => None,
        })
        .collect();
    let ports: Vec<String> = module.ports.iter().map(|p| p.name.text.clone()).collect();
    let exports = exports(compat, module, &values, &unions, &aliases, &binops, &ports)?;
    let mut annotations: BTreeMap<String, Annotation> = annotations;
    let mut values = values;
    if matches!(module.kind, hir::ModuleKind::Port) {
        for (name, annotation) in ports_annotated {
            values.push(name.clone());
            annotations.insert(name, annotation);
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

/// A top-level value canonicalized: the parts of upstream's `NodeOne` the
/// interface and the cycle checks read.
#[derive(Clone, Debug)]
pub struct ValueNode {
    pub name: String,
    pub has_params: bool,
    pub annotation: Option<Annotation>,
    /// The locals and top-level values its body uses.
    pub free: FreeLocals,
    pub def: CDef,
}

/// `toNodeOne`: a value canonicalized; none for a destructuring, which
/// elm rejects at the top level.
fn value(env: &Env<'_>, module: &hir::Module, id: hir::DefId) -> R<Option<ValueNode>> {
    let Def::Define {
        name,
        params,
        body,
        annotation,
        ..
    } = &module.defs[id]
    else {
        return Ok(None);
    };
    let (def, free) = expr::definition(env, module, name, params, *body, *annotation)?;
    // Canonicalized once more for the interface; it went through cleanly
    // above.
    let annotation = match annotation {
        Some(ty) => Some(types::to_annotation(env, module, *ty)?),
        None => None,
    };
    Ok(Some(ValueNode {
        name: name.text.clone(),
        has_params: !params.is_empty(),
        annotation,
        free,
        def,
    }))
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
        match dealias(ty) {
            Type::Lambda(arg, result) => {
                let pattern = pattern::canonicalize(env, module, param, bindings)?;
                typed.push((pattern, *arg));
                ty = *result;
            }
            _ => {
                let range = |id| {
                    module
                        .source_map
                        .pattern(id)
                        .map(|p| p.range)
                        .unwrap_or_default()
                };
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
    }
    Ok((typed, ty))
}

/// `iteratedDealias`: what `ty` is under its aliases.
fn dealias(ty: Type) -> Type {
    match ty {
        Type::Alias(_, _, args, AliasType::Holey(body)) => dealias(substitute(*body, &args)),
        Type::Alias(_, _, _, AliasType::Filled(body)) => dealias(*body),
        ty => ty,
    }
}

/// `ty` with each variable `args` names replaced.
fn substitute(ty: Type, args: &[(String, Type)]) -> Type {
    let go = |t: Type| substitute(t, args);
    match ty {
        Type::Var(name) => match args.iter().find(|(var, _)| *var == name) {
            Some((_, arg)) => arg.clone(),
            None => Type::Var(name),
        },
        Type::Lambda(a, b) => Type::Lambda(Box::new(go(*a)), Box::new(go(*b))),
        Type::Type(home, name, targs) => {
            Type::Type(home, name, targs.into_iter().map(go).collect())
        }
        Type::Record(fields, ext) => Type::Record(
            fields
                .into_iter()
                .map(|(n, f)| {
                    (
                        n,
                        crate::canonical::FieldType {
                            order: f.order,
                            ty: go(f.ty),
                        },
                    )
                })
                .collect(),
            ext,
        ),
        Type::Unit => Type::Unit,
        Type::Tuple(a, b, c) => Type::Tuple(
            Box::new(go(*a)),
            Box::new(go(*b)),
            c.map(|c| Box::new(go(*c))),
        ),
        Type::Alias(home, name, aargs, aliased) => Type::Alias(
            home,
            name,
            aargs.into_iter().map(|(v, t)| (v, go(t))).collect(),
            aliased,
        ),
    }
}

/// `canonicalizeExports`.
fn exports(
    compat: Compat,
    module: &hir::Module,
    values: &[String],
    unions: &Unions,
    aliases: &Aliases,
    binops: &BTreeMap<String, Binop>,
    ports: &[String],
) -> R<Exports> {
    let Exposing::Explicit(items) = &module.exports else {
        return Ok(Exports::Everything);
    };
    let port_module = matches!(module.kind, hir::ModuleKind::Port);
    let ok = |name: &str, range: TextRange, export| Ok(Dict::one(name, range, (range, export)));
    let infos = all(items.iter().map(|item| match item {
        Exposed::Lower(value) => {
            if values.contains(&value.text) {
                ok(&value.text, value.range, Export::Value)
            } else if port_module && ports.contains(&value.text) {
                ok(&value.text, value.range, Export::Port)
            } else {
                // `ports ++ Map.keys values`, each in name order.
                let sorted = |names: &[String]| -> Vec<String> {
                    super::sorted(compat, names.iter())
                        .into_iter()
                        .cloned()
                        .collect()
                };
                let mut possible = if port_module {
                    sorted(ports)
                } else {
                    Vec::new()
                };
                possible.extend(sorted(values));
                throw(Error::ExportNotFound {
                    range: value.range,
                    kind: NameKind::Value,
                    name: value.text.clone(),
                    possible,
                })
            }
        }
        Exposed::Operator(op) => {
            if binops.contains_key(&op.text) {
                ok(&op.text, op.range, Export::Binop)
            } else {
                throw(Error::ExportNotFound {
                    range: op.range,
                    kind: NameKind::Operator,
                    name: op.text.clone(),
                    possible: super::sorted(compat, binops.keys())
                        .into_iter()
                        .cloned()
                        .collect(),
                })
            }
        }
        Exposed::Upper(name, open) => {
            let types = || {
                let unions = super::sorted(compat, unions.keys());
                unions
                    .into_iter()
                    .chain(super::sorted(compat, aliases.keys()))
                    .cloned()
                    .collect()
            };
            match open {
                Some(_) if unions.contains_key(&name.text) => {
                    ok(&name.text, name.range, Export::UnionOpen)
                }
                Some(dots) if aliases.contains_key(&name.text) => throw(Error::ExportOpenAlias {
                    range: *dots,
                    name: name.text.clone(),
                }),
                None if unions.contains_key(&name.text) => {
                    ok(&name.text, name.range, Export::UnionClosed)
                }
                None if aliases.contains_key(&name.text) => {
                    ok(&name.text, name.range, Export::Alias)
                }
                _ => throw(Error::ExportNotFound {
                    range: name.range,
                    kind: NameKind::Type,
                    name: name.text.clone(),
                    possible: types(),
                }),
            }
        }
    }))?;
    let kinds: HashMap<String, NameKind> = items
        .iter()
        .map(|item| match item {
            Exposed::Lower(v) => (v.text.clone(), NameKind::Value),
            Exposed::Operator(o) => (o.text.clone(), NameKind::Operator),
            Exposed::Upper(t, _) => (t.text.clone(), NameKind::Type),
        })
        .collect();
    let exports =
        Dict::unions(infos).detect(compat, |name, first, second| Error::ExportDuplicate {
            kind: kinds.get(name).copied().unwrap_or(NameKind::Value),
            name: name.to_owned(),
            first,
            second,
        })?;
    Ok(Exports::Explicit(
        exports
            .into_iter()
            .map(|(name, (_, export))| (name, export))
            .collect(),
    ))
}

/// The annotation of a value whose type nomi doesn't infer yet (M6): its
/// name is all an importer needs to canonicalize.
fn unknown() -> Annotation {
    Annotation {
        free: Vec::new(),
        ty: Type::Var("?".to_owned()),
    }
}

/// `fromModule`: what the module offers importers.
pub fn interface(canonical: &Canonical) -> Interface {
    let annotation = |name: &str| {
        canonical
            .annotations
            .get(name)
            .cloned()
            .unwrap_or_else(unknown)
    };
    let exported = |name: &str| match &canonical.exports {
        Exports::Everything => Some(None),
        Exports::Explicit(exports) => exports.get(name).map(Some),
    };
    let values = (canonical.values.iter())
        .filter(|name| exported(name).is_some())
        .map(|name| (name.clone(), annotation(name)))
        .collect();
    let unions = (canonical.unions.iter())
        .map(|(name, union)| {
            let union = match exported(name) {
                Some(None | Some(Export::UnionOpen)) => interface::Union::Open(union.clone()),
                Some(Some(_)) => interface::Union::Closed(union.clone()),
                None => interface::Union::Private(union.clone()),
            };
            (name.clone(), union)
        })
        .collect();
    let aliases = (canonical.aliases.iter())
        .map(|(name, alias)| {
            let alias = match exported(name) {
                Some(_) => interface::Alias::Public(alias.clone()),
                None => interface::Alias::Private(alias.clone()),
            };
            (name.clone(), alias)
        })
        .collect();
    let binops = (canonical.binops.iter())
        .filter(|(op, _)| exported(op).is_some())
        .map(|(op, b)| {
            let binop = interface::Binop {
                function: b.function.clone(),
                annotation: annotation(&b.function),
                associativity: b.associativity,
                precedence: b.precedence,
            };
            (op.clone(), binop)
        })
        .collect();
    Interface {
        home: canonical.home.package.clone(),
        values,
        unions,
        aliases,
        binops,
    }
}
