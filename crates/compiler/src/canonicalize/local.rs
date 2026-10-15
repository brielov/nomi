//! The module's own types, aliases, constructors and values, added over
//! the imports (`Environment/Local.hs`): first the types, then the
//! values, then the constructors, each stopping the rest when it fails.
//! Declarations are taken in reverse source order, as upstream's parser
//! leaves them.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use nomi_base::{Compat, TextRange, grow};

use super::dups::Dict;
use super::env::{CtorInfo, Env, TypeInfo, Var};
use super::error::{Declaration, Duplicated, Error, R, all, throw};
use super::types;
use crate::canonical::{self, Ctor, CtorOpts, Home, Type};
use crate::graph::{Node, Scc, strongly_connected};
use crate::hir::{self, ModuleKind, Spanned};

pub(super) type Unions = BTreeMap<String, Arc<canonical::Union>>;
pub(super) type Aliases = BTreeMap<String, canonical::Alias>;

/// The environment with the module's own declarations, and its types.
pub(super) struct Local<'a> {
    pub(super) env: Env<'a>,
    pub(super) unions: Unions,
    pub(super) aliases: Aliases,
}

/// `Local.add`.
pub(super) fn add<'a>(module: &hir::Module, env: Env<'a>) -> R<Local<'a>> {
    let env = add_types(module, env)?;
    let env = add_vars(module, env)?;
    add_ctors(module, env)
}

/// `addVars`: the top-level values and ports, which replace imported
/// values of their names.
fn add_vars<'a>(module: &hir::Module, mut env: Env<'a>) -> R<Env<'a>> {
    let mut dups = Dict::default();
    match &module.kind {
        ModuleKind::Effect(manager) => {
            for (name, ty) in manager.given() {
                dups.insert(env.compat, name, ty.range, Var::TopLevel(ty.range));
            }
        }
        ModuleKind::Port | ModuleKind::Plain => {
            for port in module.ports.iter().rev() {
                let name = &port.name;
                dups.insert(
                    env.compat,
                    &name.text,
                    name.range,
                    Var::TopLevel(name.range),
                );
            }
        }
    }
    for name in module.value_names().rev() {
        dups.insert(
            env.compat,
            &name.text,
            name.range,
            Var::TopLevel(name.range),
        );
    }
    let vars = dups.detect(env.compat, Duplicated::Decl.error())?;
    Rc::make_mut(&mut env.vars).extend(vars);
    Ok(env)
}

/// `addTypes`: unions, then aliases in dependency order, after checking
/// no two types share a name.
fn add_types<'a>(module: &hir::Module, mut env: Env<'a>) -> R<Env<'a>> {
    let compat = env.compat;
    let mut dups = Dict::default();
    for alias in module.aliases.iter().rev() {
        dups.insert(compat, &alias.name.text, alias.name.range, ());
    }
    for union in module.unions.iter().rev() {
        dups.insert(compat, &union.name.text, union.name.range, ());
    }
    dups.detect(compat, Duplicated::Type.error())?;
    let home = env.home.clone();
    for union in module.unions.iter().rev() {
        let arity = check_union_free_vars(compat, module, union)?;
        let info = TypeInfo::Union {
            arity,
            home: home.clone(),
        };
        let types = Rc::make_mut(&mut env.types);
        types.insert(
            union.name.text.clone(),
            BTreeMap::from([(home.clone(), info)]),
        );
    }
    let nodes = (module.aliases.iter().rev())
        .map(|alias| Node {
            value: alias,
            key: alias.name.text.clone(),
            edges: edges(module, alias.body),
        })
        .collect();
    for scc in strongly_connected(compat, nodes) {
        match scc {
            Scc::Acyclic(Node { value: alias, .. }) => {
                let vars = check_alias_free_vars(compat, module, alias)?;
                let ty = types::canonicalize(&env, module, alias.body)?;
                let info = TypeInfo::Alias {
                    home: home.clone(),
                    vars,
                    ty: ty.into(),
                };
                let types = Rc::make_mut(&mut env.types);
                types.insert(
                    alias.name.text.clone(),
                    BTreeMap::from([(home.clone(), info)]),
                );
            }
            Scc::Cyclic(aliases) => {
                let alias = aliases[0].value;
                let args = check_alias_free_vars(compat, module, alias)?;
                return throw(Error::RecursiveAlias {
                    range: alias.name.range,
                    name: alias.name.text.clone(),
                    args,
                    body: alias.body,
                    others: aliases[1..].iter().map(|a| a.key.clone()).collect(),
                });
            }
        }
    }
    Ok(env)
}
/// `getEdges`: the unqualified type names `ty` mentions, last met first.
fn edges(module: &hir::Module, ty: hir::TypeId) -> Vec<String> {
    fn walk(module: &hir::Module, ty: hir::TypeId, met: &mut Vec<String>) {
        grow(|| match &module.types[ty] {
            hir::Type::Lambda(a, b) => {
                walk(module, *a, met);
                walk(module, *b, met);
            }
            hir::Type::Named {
                qualifier,
                name,
                args,
                ..
            } => {
                if qualifier.is_none() {
                    met.push(name.text.clone());
                }
                for &a in args {
                    walk(module, a, met);
                }
            }
            hir::Type::Record { fields, .. } => {
                for &(_, t) in fields {
                    walk(module, t, met);
                }
            }
            hir::Type::Tuple(items) => items.iter().for_each(|&t| walk(module, t, met)),
            hir::Type::Var(_) | hir::Type::Unit | hir::Type::Missing => {}
        });
    }
    let mut met = Vec::new();
    walk(module, ty, &mut met);
    met.reverse();
    met
}

/// `addFreeVars`: each type variable `ty` names, at the place upstream
/// keeps: a later place replaces an earlier one.
fn add_free_vars(module: &hir::Module, ty: hir::TypeId, vars: &mut BTreeMap<String, TextRange>) {
    let range = module.source_map.type_range(ty);
    grow(|| match &module.types[ty] {
        hir::Type::Lambda(a, b) => {
            add_free_vars(module, *a, vars);
            add_free_vars(module, *b, vars);
        }
        hir::Type::Var(name) => {
            vars.insert(name.clone(), range);
        }
        hir::Type::Named { args, .. } => args.iter().for_each(|&a| add_free_vars(module, a, vars)),
        hir::Type::Record { fields, extends } => {
            if let Some(ext) = extends {
                vars.insert(ext.text.clone(), ext.range);
            }
            for &(_, t) in fields {
                add_free_vars(module, t, vars);
            }
        }
        hir::Type::Tuple(items) => items.iter().for_each(|&t| add_free_vars(module, t, vars)),
        hir::Type::Unit | hir::Type::Missing => {}
    });
}

/// The parameters of a declaration, each written once.
fn bound_vars(
    compat: Compat,
    declaration: Declaration,
    owner: &str,
    params: &[Spanned],
) -> R<BTreeMap<String, TextRange>> {
    let mut dict = Dict::default();
    for param in params.iter().rev() {
        dict.insert(compat, &param.text, param.range, param.range);
    }
    dict.detect(compat, |name, first, second| Error::DuplicateArg {
        declaration,
        owner: owner.to_owned(),
        name: name.to_owned(),
        first,
        second,
    })
}

/// The variables of `vars` missing from `from`, in name order.
fn difference(
    compat: Compat,
    vars: &BTreeMap<String, TextRange>,
    from: &BTreeMap<String, TextRange>,
) -> Vec<Spanned> {
    (compat
        .sorted_names(vars.keys().filter(|v| !from.contains_key(*v)))
        .into_iter())
    .map(|name| Spanned {
        text: name.clone(),
        range: vars[name],
    })
    .collect()
}

fn texts(params: &[Spanned]) -> Vec<String> {
    params.iter().map(|p| p.text.clone()).collect()
}

/// `checkUnionFreeVars`: the union's arity, if every variable its
/// constructors use is a parameter.
fn check_union_free_vars(compat: Compat, module: &hir::Module, union: &hir::Union) -> R<usize> {
    let name = &union.name.text;
    let bound = bound_vars(compat, Declaration::Union, name, &union.params)?;
    // Constructors last first, as upstream's `foldr`, so the first
    // constructor to use a variable places it.
    let mut free = BTreeMap::new();
    for (_, args) in union.variants.iter().rev() {
        for &arg in args {
            add_free_vars(module, arg, &mut free);
        }
    }
    let mut unbound = difference(compat, &free, &bound).into_iter();
    match unbound.next() {
        None => Ok(union.params.len()),
        Some(first) => throw(Error::TypeVarsUnboundInUnion {
            range: union.range,
            name: name.clone(),
            vars: texts(&union.params),
            first,
            rest: unbound.collect(),
        }),
    }
}

/// `checkAliasFreeVars`: the alias's parameters, if its body uses each
/// and no other.
fn check_alias_free_vars(
    compat: Compat,
    module: &hir::Module,
    alias: &hir::Alias,
) -> R<Vec<String>> {
    let name = &alias.name.text;
    let bound = bound_vars(compat, Declaration::Alias, name, &alias.params)?;
    let mut free = BTreeMap::new();
    add_free_vars(module, alias.body, &mut free);
    let unused = difference(compat, &bound, &free);
    let unbound = difference(compat, &free, &bound);
    let vars = texts(&alias.params);
    if unused.is_empty() && unbound.is_empty() {
        Ok(vars)
    } else {
        throw(Error::TypeVarsMessedUpInAlias {
            range: alias.range,
            name: name.clone(),
            vars,
            unused,
            unbound,
        })
    }
}

/// `addCtors`: every union's constructors and every record alias's, each
/// name once, replacing imported constructors of their names.
fn add_ctors<'a>(module: &hir::Module, mut env: Env<'a>) -> R<Local<'a>> {
    let unions = all(module
        .unions
        .iter()
        .rev()
        .map(|u| canonicalize_union(module, u, &env)))?;
    let mut dups = Dict::default();
    let unions = by_name(unions, &mut dups);
    let aliases = (module.aliases.iter().rev()).map(|a| alias_ctor(a, &env));
    let aliases = by_name(aliases, &mut dups);
    let ctors = dups.detect(env.compat, Duplicated::Ctor.error())?;
    Rc::make_mut(&mut env.ctors).extend(ctors);
    Ok(Local {
        env,
        unions,
        aliases,
    })
}

type CtorDups<'a> = Dict<BTreeMap<Home, CtorInfo<'a>>>;

/// A type canonicalized, and the constructors it declares.
struct Declared<'a, V> {
    name: String,
    value: V,
    ctors: CtorDups<'a>,
}

/// The declarations by name, their constructors added to `dups`.
fn by_name<'a, V>(
    declared: impl IntoIterator<Item = Declared<'a, V>>,
    dups: &mut CtorDups<'a>,
) -> BTreeMap<String, V> {
    (declared.into_iter())
        .map(|d| {
            *dups = std::mem::take(dups).union(d.ctors);
            (d.name, d.value)
        })
        .collect()
}

fn canonicalize_union<'a>(
    module: &hir::Module,
    union: &hir::Union,
    env: &Env<'a>,
) -> R<Declared<'a, Arc<canonical::Union>>> {
    let ctors = all(union.variants.iter().map(|(name, args)| {
        let args = all(args.iter().map(|&a| types::canonicalize(env, module, a)))?;
        Ok(Ctor {
            name: name.text.clone(),
            args,
        })
    }))?;
    let opts = match &union.variants[..] {
        [(_, args)] if args.len() == 1 => CtorOpts::Unbox,
        variants if variants.iter().all(|(_, args)| args.is_empty()) => CtorOpts::Enum,
        _ => CtorOpts::Normal,
    };
    let canonical = Arc::new(canonical::Union {
        vars: texts(&union.params),
        ctors,
        opts,
    });
    let home = &env.home;
    let ctors = Dict::unions(union.variants.iter().enumerate().map(|(at, (name, _))| {
        let info = CtorInfo::Ctor {
            home: home.clone(),
            union_name: union.name.text.clone(),
            union: Cow::Owned(Arc::clone(&canonical)),
            ctor: at,
        };
        Dict::one(
            &name.text,
            name.range,
            BTreeMap::from([(home.clone(), info)]),
        )
    }));
    Ok(Declared {
        name: union.name.text.clone(),
        value: canonical,
        ctors,
    })
}

/// The alias `addTypes` canonicalized, and the constructor of a record
/// alias.
fn alias_ctor<'a>(alias: &hir::Alias, env: &Env<'a>) -> Declared<'a, canonical::Alias> {
    let home = &env.home;
    let name = &alias.name.text;
    let Some(TypeInfo::Alias { vars, ty, .. }) = env.types.get(name).and_then(|t| t.get(home))
    else {
        unreachable!("`addTypes` adds every alias of the module");
    };
    let ctors = match &**ty {
        Type::Record(fields, None) => {
            let info = CtorInfo::record(env.compat, home, name, vars, fields, ty.clone());
            let ctor = BTreeMap::from([(home.clone(), info)]);
            Dict::one(name, alias.name.range, ctor)
        }
        _ => Dict::default(),
    };
    let canonical = canonical::Alias {
        vars: vars.clone(),
        ty: ty.clone(),
    };
    Declared {
        name: name.clone(),
        value: canonical,
        ctors,
    }
}
