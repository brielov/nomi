//! The module's own types, aliases, constructors and values, added over
//! the imports (`Environment/Local.hs`): first the types, then the
//! values, then the constructors, each stopping the rest when it fails.
//! Declarations are taken in reverse source order, as upstream's parser
//! leaves them.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::rc::Rc;

use nomi_base::TextRange;

use super::dups::Dict;
use super::env::{CtorInfo, Env, TypeInfo, Var};
use super::error::{At, Error, R, all, throw};
use super::types;
use crate::canonical::{self, AliasType, Ctor, CtorOpts, Home, Type};
use crate::graph::{Scc, strongly_connected};
use crate::hir::{self, Def, ModuleKind};

pub type Unions = BTreeMap<String, canonical::Union>;
pub type Aliases = BTreeMap<String, canonical::Alias>;

/// `Local.add`.
pub fn add<'a>(module: &hir::Module, env: Env<'a>) -> R<(Env<'a>, Unions, Aliases)> {
    let env = add_types(module, env)?;
    let env = add_vars(module, env)?;
    add_ctors(module, env)
}

/// `addVars`: the top-level values and ports, which replace imported
/// values of their names.
fn add_vars<'a>(module: &hir::Module, mut env: Env<'a>) -> R<Env<'a>> {
    let mut dups = Dict::default();
    match &module.kind {
        ModuleKind::Effect(fields) => {
            let mut manager = Dict::default();
            for (field, ty) in fields {
                let name = match field.text.as_str() {
                    "command" => "command",
                    "subscription" => "subscription",
                    _ => continue,
                };
                manager = manager.union(Dict::one(name, ty.range, Var::TopLevel(ty.range)));
            }
            dups = manager;
        }
        ModuleKind::Port | ModuleKind::Plain => {
            for port in module.ports.iter().rev() {
                dups.insert(
                    env.compat,
                    &port.name.text,
                    port.name.range,
                    Var::TopLevel(port.name.range),
                );
            }
        }
    }
    for &id in module.values.iter().rev() {
        if let Def::Define { name, .. } = &module.defs[id] {
            dups.insert(
                env.compat,
                &name.text,
                name.range,
                Var::TopLevel(name.range),
            );
        }
    }
    let vars = dups.detect(env.compat, |name, first, second| Error::DuplicateDecl {
        name: name.to_owned(),
        first,
        second,
    })?;
    Rc::make_mut(&mut env.vars).extend(vars);
    Ok(env)
}

/// `addTypes`: unions, then aliases in dependency order, after checking
/// no two types share a name.
fn add_types<'a>(module: &hir::Module, mut env: Env<'a>) -> R<Env<'a>> {
    let mut dups = Dict::default();
    for alias in module.aliases.iter().rev() {
        dups.insert(env.compat, &alias.name.text, alias.name.range, ());
    }
    for union in module.unions.iter().rev() {
        dups.insert(env.compat, &union.name.text, union.name.range, ());
    }
    dups.detect(env.compat, |name, first, second| Error::DuplicateType {
        name: name.to_owned(),
        first,
        second,
    })?;
    let home = env.home.clone();
    for union in module.unions.iter().rev() {
        let arity = check_union_free_vars(module, union, &env)?;
        Rc::make_mut(&mut env.types).insert(
            union.name.text.clone(),
            BTreeMap::from([(
                home.clone(),
                TypeInfo::Union {
                    arity,
                    home: home.clone(),
                },
            )]),
        );
    }
    let nodes = (module.aliases.iter().rev())
        .map(|alias| (alias, alias.name.text.clone(), edges(module, alias.body)))
        .collect();
    let order = super::name_order(env.compat);
    for scc in strongly_connected(nodes, |a: &String, b: &String| order(a, b)) {
        match scc {
            Scc::Acyclic(alias) => {
                let args = check_alias_free_vars(module, alias, &env)?;
                let ty = types::canonicalize(&env, module, alias.body)?;
                let info = TypeInfo::Alias {
                    arity: args.len(),
                    home: home.clone(),
                    vars: args,
                    ty: ty.into(),
                };
                Rc::make_mut(&mut env.types).insert(
                    alias.name.text.clone(),
                    BTreeMap::from([(home.clone(), info)]),
                );
            }
            Scc::Cyclic(aliases) => {
                let alias = aliases[0];
                let args = check_alias_free_vars(module, alias, &env)?;
                return throw(Error::RecursiveAlias {
                    range: alias.name.range,
                    name: alias.name.text.clone(),
                    args,
                    body: alias.body,
                    others: aliases[1..].iter().map(|a| a.name.text.clone()).collect(),
                });
            }
        }
    }
    Ok(env)
}

/// `getEdges`: the unqualified type names `ty` mentions, last met first.
fn edges(module: &hir::Module, ty: hir::TypeId) -> Vec<String> {
    fn walk(module: &hir::Module, ty: hir::TypeId, met: &mut Vec<String>) {
        match &module.types[ty] {
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
        }
    }
    let mut met = Vec::new();
    walk(module, ty, &mut met);
    met.reverse();
    met
}

/// `addFreeVars`: each type variable `ty` names, at the place upstream
/// keeps: a later place replaces an earlier one.
fn add_free_vars(module: &hir::Module, ty: hir::TypeId, vars: &mut BTreeMap<String, TextRange>) {
    let range = module
        .source_map
        .ty(ty)
        .map(|p| p.range)
        .unwrap_or_default();
    match &module.types[ty] {
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
    }
}

/// The parameters of a declaration, each written once.
fn bound_vars(
    params: &[hir::Spanned],
    env: &Env<'_>,
    error: impl Fn(&str, TextRange, TextRange) -> Error,
) -> R<BTreeMap<String, TextRange>> {
    let mut dict = Dict::default();
    for param in params.iter().rev() {
        dict.insert(env.compat, &param.text, param.range, param.range);
    }
    dict.detect(env.compat, error)
}

/// The variables of `vars` missing from `from`, in name order.
fn difference(
    env: &Env<'_>,
    vars: &BTreeMap<String, TextRange>,
    from: &BTreeMap<String, TextRange>,
) -> Vec<At> {
    super::sorted(env.compat, vars.keys().filter(|v| !from.contains_key(*v)))
        .into_iter()
        .map(|name| At {
            name: name.clone(),
            range: vars[name],
        })
        .collect()
}

/// `checkUnionFreeVars`: the union's arity, if every variable its
/// constructors use is a parameter.
fn check_union_free_vars(module: &hir::Module, union: &hir::Union, env: &Env<'_>) -> R<usize> {
    let name = &union.name.text;
    let bound = bound_vars(&union.params, env, |var, first, second| {
        Error::DuplicateUnionArg {
            union: name.clone(),
            name: var.to_owned(),
            first,
            second,
        }
    })?;
    // Constructors last first, as upstream's `foldr`, so the first
    // constructor to use a variable places it.
    let mut free = BTreeMap::new();
    for (_, args) in union.variants.iter().rev() {
        let mut ctor = free.clone();
        for &arg in args {
            add_free_vars(module, arg, &mut ctor);
        }
        free = ctor;
    }
    match difference(env, &free, &bound).split_first() {
        None => Ok(union.params.len()),
        Some((first, rest)) => throw(Error::TypeVarsUnboundInUnion {
            range: union.range,
            name: name.clone(),
            vars: union.params.iter().map(|p| p.text.clone()).collect(),
            first: first.clone(),
            rest: rest.to_vec(),
        }),
    }
}

/// `checkAliasFreeVars`: the alias's parameters, if its body uses each
/// and no other.
fn check_alias_free_vars(
    module: &hir::Module,
    alias: &hir::Alias,
    env: &Env<'_>,
) -> R<Vec<String>> {
    let name = &alias.name.text;
    let bound = bound_vars(&alias.params, env, |var, first, second| {
        Error::DuplicateAliasArg {
            alias: name.clone(),
            name: var.to_owned(),
            first,
            second,
        }
    })?;
    let mut free = BTreeMap::new();
    add_free_vars(module, alias.body, &mut free);
    let unused = difference(env, &bound, &free);
    let unbound = difference(env, &free, &bound);
    let vars: Vec<String> = alias.params.iter().map(|p| p.text.clone()).collect();
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
fn add_ctors<'a>(module: &hir::Module, mut env: Env<'a>) -> R<(Env<'a>, Unions, Aliases)> {
    let unions = all(module
        .unions
        .iter()
        .rev()
        .map(|u| canonicalize_union(module, u, &env)))?;
    let aliases = all(module
        .aliases
        .iter()
        .rev()
        .map(|a| canonicalize_alias(module, a, &env)))?;
    let union_dups = Dict::unions(unions.iter().map(|(_, d)| d.clone()));
    let alias_dups = Dict::unions(aliases.iter().map(|(_, d)| d.clone()));
    let ctors = union_dups
        .union(alias_dups)
        .detect(env.compat, |name, first, second| Error::DuplicateCtor {
            name: name.to_owned(),
            first,
            second,
        })?;
    Rc::make_mut(&mut env.ctors).extend(ctors);
    Ok((
        env,
        unions.into_iter().map(|(u, _)| u).collect(),
        aliases.into_iter().map(|(a, _)| a).collect(),
    ))
}

type CtorDups<'a> = Dict<BTreeMap<Home, CtorInfo<'a>>>;

fn canonicalize_union<'a>(
    module: &hir::Module,
    union: &hir::Union,
    env: &Env<'a>,
) -> R<((String, canonical::Union), CtorDups<'a>)> {
    let ctors = all(union
        .variants
        .iter()
        .enumerate()
        .map(|(index, (name, args))| {
            let args = all(args.iter().map(|&a| types::canonicalize(env, module, a)))?;
            Ok(Ctor {
                name: name.text.clone(),
                index,
                args,
            })
        }))?;
    let opts = match &union.variants[..] {
        [(_, args)] if args.len() == 1 => CtorOpts::Unbox,
        variants if variants.iter().all(|(_, args)| args.is_empty()) => CtorOpts::Enum,
        _ => CtorOpts::Normal,
    };
    let canonical = canonical::Union {
        vars: union.params.iter().map(|p| p.text.clone()).collect(),
        count: ctors.len(),
        ctors,
        opts,
    };
    let home = env.home.clone();
    let dups = Dict::unions(union.variants.iter().zip(&canonical.ctors).map(
        |((name, _), ctor)| {
            let info = CtorInfo::Ctor {
                home: home.clone(),
                union_name: union.name.text.clone(),
                union: Cow::Owned(canonical.clone()),
                index: ctor.index,
                args: Cow::Owned(ctor.args.clone()),
            };
            Dict::one(
                &name.text,
                name.range,
                BTreeMap::from([(home.clone(), info)]),
            )
        },
    ));
    Ok(((union.name.text.clone(), canonical), dups))
}

fn canonicalize_alias<'a>(
    module: &hir::Module,
    alias: &hir::Alias,
    env: &Env<'a>,
) -> R<((String, canonical::Alias), CtorDups<'a>)> {
    let vars: Vec<String> = alias.params.iter().map(|p| p.text.clone()).collect();
    let ty = types::canonicalize(env, module, alias.body)?;
    let home = env.home.clone();
    let name = &alias.name.text;
    let dups = match &ty {
        Type::Record(fields, None) => {
            let args = vars
                .iter()
                .map(|v| (v.clone(), Type::Var(v.clone())))
                .collect();
            let result = Type::Alias(
                home.clone(),
                name.clone(),
                args,
                AliasType::Filled(std::sync::Arc::new(Type::Record(fields.clone(), None))),
            );
            let mut by_order: Vec<_> = fields.values().collect();
            by_order.sort_by_key(|f| f.order);
            let ty = by_order.into_iter().rev().fold(result, |acc, f| {
                Type::Lambda(Box::new(f.ty.clone()), Box::new(acc))
            });
            let info = CtorInfo::Record {
                home: home.clone(),
                vars: vars.clone(),
                ty,
            };
            Dict::one(name, alias.name.range, BTreeMap::from([(home, info)]))
        }
        _ => Dict::default(),
    };
    Ok((
        (
            name.clone(),
            canonical::Alias {
                vars,
                ty: ty.into(),
            },
        ),
        dups,
    ))
}
