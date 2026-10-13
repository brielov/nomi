//! Canonicalizing types (`Canonicalize/Type.hs`): each name resolved to
//! the type or alias it means, with its arity checked.

use std::collections::{BTreeMap, BTreeSet};

use super::dups::check_fields;
use super::env::{Env, TypeInfo};
use super::error::{ArityContext, Error, R, all, both, throw};
use crate::canonical::{AliasType, Annotation, FieldType, Type};
use crate::hir::{self, TypeId};

/// `toAnnotation`: the type and its free variables.
pub fn to_annotation(env: &Env<'_>, module: &hir::Module, id: TypeId) -> R<Annotation> {
    let ty = canonicalize(env, module, id)?;
    let mut free = BTreeSet::new();
    free_vars(&ty, &mut free);
    Ok(Annotation {
        free: free.into_iter().collect(),
        ty: ty.into(),
    })
}

/// `canonicalize`.
pub fn canonicalize(env: &Env<'_>, module: &hir::Module, id: TypeId) -> R<Type> {
    stacker::maybe_grow(128 * 1024, 2 * 1024 * 1024, || {
        canonicalize_(env, module, id)
    })
}

fn canonicalize_(env: &Env<'_>, module: &hir::Module, id: TypeId) -> R<Type> {
    let range = module
        .source_map
        .ty(id)
        .map(|p| p.range)
        .unwrap_or_default();
    match &module.types[id] {
        // Lowering leaves this only where the syntax is wrong, which a
        // module canonicalized has no part of.
        hir::Type::Missing => Ok(Type::Unit),
        hir::Type::Var(name) => Ok(Type::Var(name.clone())),
        hir::Type::Named {
            qualifier,
            name,
            args,
        } => {
            let info = env.find_type(name.range, qualifier.as_deref(), &name.text)?;
            let cargs = all(args.iter().map(|&a| canonicalize(env, module, a)))?;
            let (arity, ty) = match info {
                TypeInfo::Alias {
                    arity,
                    home,
                    vars,
                    ty,
                } => {
                    let pairs = vars.iter().cloned().zip(cargs).collect();
                    let holey = AliasType::Holey(ty.clone());
                    (
                        *arity,
                        Type::Alias(home.clone(), name.text.clone(), pairs, holey),
                    )
                }
                TypeInfo::Union { arity, home } => {
                    (*arity, Type::Type(home.clone(), name.text.clone(), cargs))
                }
            };
            if arity == args.len() {
                Ok(ty)
            } else {
                throw(Error::BadArity {
                    range,
                    context: ArityContext::Type,
                    name: name.text.clone(),
                    expected: arity,
                    actual: args.len(),
                })
            }
        }
        hir::Type::Lambda(a, b) => {
            let (a, b) = both(canonicalize(env, module, *a), canonicalize(env, module, *b))?;
            Ok(Type::Lambda(Box::new(a), Box::new(b)))
        }
        hir::Type::Record { fields, extends } => {
            // The fields are checked for duplicates before any is
            // canonicalized, and then canonicalized in name order.
            let fields = (fields.iter().enumerate())
                .map(|(i, (name, ty))| (name.text.clone(), name.range, (i, *ty)))
                .collect();
            let fields = check_fields(env.compat, fields)?;
            let names: Vec<String> = super::sorted(env.compat, fields.keys())
                .into_iter()
                .cloned()
                .collect();
            let types = all(names.iter().map(|name| {
                let (i, ty) = fields[name];
                let order = u16::try_from(i).unwrap_or(u16::MAX);
                canonicalize(env, module, ty).map(|ty| FieldType { order, ty })
            }))?;
            let fields: BTreeMap<String, FieldType> = names.into_iter().zip(types).collect();
            Ok(Type::Record(
                fields,
                extends.as_ref().map(|e| e.text.clone()),
            ))
        }
        hir::Type::Unit => Ok(Type::Unit),
        hir::Type::Tuple(items) => {
            let (a, b) = (items[0], items[1]);
            let rest = match &items[2..] {
                [] => Ok(None),
                [c] => canonicalize(env, module, *c).map(Some),
                _ => throw(Error::TupleLargerThanThree { range }),
            };
            let ((a, b), c) = both(
                both(canonicalize(env, module, a), canonicalize(env, module, b)),
                rest,
            )?;
            Ok(Type::Tuple(Box::new(a), Box::new(b), c.map(Box::new)))
        }
    }
}

/// `addFreeVars`: the type variables `ty` names.
pub fn free_vars(ty: &Type, vars: &mut BTreeSet<String>) {
    match ty {
        Type::Lambda(a, b) => {
            free_vars(b, vars);
            free_vars(a, vars);
        }
        Type::Var(name) => {
            vars.insert(name.clone());
        }
        Type::Type(_, _, args) => args.iter().for_each(|a| free_vars(a, vars)),
        Type::Record(fields, ext) => {
            if let Some(ext) = ext {
                vars.insert(ext.clone());
            }
            for f in fields.values() {
                free_vars(&f.ty, vars);
            }
        }
        Type::Unit => {}
        Type::Tuple(a, b, c) => {
            free_vars(a, vars);
            free_vars(b, vars);
            if let Some(c) = c {
                free_vars(c, vars);
            }
        }
        Type::Alias(_, _, args, _) => args.iter().for_each(|(_, a)| free_vars(a, vars)),
    }
}
