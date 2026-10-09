//! Ports and effect managers (`Canonicalize/Effects.hs`): a port must be
//! a command sending, or a subscription receiving, a value that can cross
//! to JavaScript; an effect manager must define the functions and types
//! its header names.

use std::collections::BTreeMap;

use nomi_base::TextRange;

use super::env::Env;
use super::error::{Error, InvalidPayload, PortProblem, R, all, both, throw};
use super::local::Unions;
use super::types;
use crate::canonical::{AliasType, Home, Type};
use crate::hir::{self, Def, ModuleKind};

/// `canonicalize`: the module's ports, in upstream's reverse order, or
/// its effect manager.
pub fn canonicalize(env: &Env<'_>, module: &hir::Module, unions: &Unions) -> R<()> {
    match &module.kind {
        ModuleKind::Plain => Ok(()),
        ModuleKind::Port => {
            all(module.ports.iter().rev().map(|p| port(env, module, p))).map(|_| ())
        }
        ModuleKind::Effect(fields) => manager(module, fields, unions),
    }
}

fn is(home: &Home, package: &str, module: &str) -> bool {
    home.package == package && home.module == module
}

/// `canonicalizePort`.
fn port(env: &Env<'_>, module: &hir::Module, port: &hir::Port) -> R<()> {
    let region = port.name.range;
    let name = &port.name.text;
    let invalid = |problem| {
        throw(Error::PortTypeInvalid {
            range: region,
            name: name.clone(),
            problem,
        })
    };
    let annotation = types::to_annotation(env, module, port.ty)?;
    let mut parts = delambda(deep_dealias(&annotation.ty));
    let Some(Type::Type(home, type_name, args)) = parts.pop() else {
        return invalid(PortProblem::NotCmdOrSub);
    };
    let [msg] = &args[..] else {
        return invalid(PortProblem::NotCmdOrSub);
    };
    let payload = |ty: &Type| match check_payload(ty) {
        Ok(()) => Ok(()),
        Err(bad) => throw(Error::PortPayloadInvalid {
            range: region,
            name: name.clone(),
            bad: bad.0,
            problem: bad.1,
        }),
    };
    if is(&home, "elm/core", "Platform.Cmd") && type_name == "Cmd" {
        match &parts[..] {
            [] => invalid(PortProblem::CmdNoArg),
            [outgoing] => match msg {
                Type::Var(_) => payload(outgoing),
                _ => invalid(PortProblem::CmdBadMsg),
            },
            _ => invalid(PortProblem::CmdExtraArgs(parts.len())),
        }
    } else if is(&home, "elm/core", "Platform.Sub") && type_name == "Sub" {
        match (&parts[..], msg) {
            ([Type::Lambda(incoming, result)], Type::Var(msg2)) if matches!(&**result, Type::Var(msg1) if msg1 == msg2) => {
                payload(incoming)
            }
            _ => invalid(PortProblem::SubBad),
        }
    } else {
        invalid(PortProblem::NotCmdOrSub)
    }
}

/// `delambda`: a function's arguments, then its result.
fn delambda(ty: Type) -> Vec<Type> {
    match ty {
        Type::Lambda(arg, result) => {
            let mut parts = vec![*arg];
            parts.extend(delambda(*result));
            parts
        }
        ty => vec![ty],
    }
}

/// `dealias`: what an alias, applied to `args`, stands for.
fn dealias(args: &[(String, Type)], aliased: &AliasType) -> Type {
    match aliased {
        AliasType::Holey(body) => substitute(body, args),
        AliasType::Filled(body) => (**body).clone(),
    }
}

fn substitute(ty: &Type, args: &[(String, Type)]) -> Type {
    match ty {
        Type::Var(name) => (args.iter())
            .find(|(var, _)| var == name)
            .map_or_else(|| ty.clone(), |(_, arg)| arg.clone()),
        Type::Lambda(a, b) => {
            Type::Lambda(Box::new(substitute(a, args)), Box::new(substitute(b, args)))
        }
        Type::Type(home, name, targs) => Type::Type(
            home.clone(),
            name.clone(),
            targs.iter().map(|t| substitute(t, args)).collect(),
        ),
        Type::Record(fields, ext) => Type::Record(
            (fields.iter())
                .map(|(n, f)| {
                    let field = crate::canonical::FieldType {
                        order: f.order,
                        ty: substitute(&f.ty, args),
                    };
                    (n.clone(), field)
                })
                .collect(),
            ext.clone(),
        ),
        Type::Unit => Type::Unit,
        Type::Tuple(a, b, c) => Type::Tuple(
            Box::new(substitute(a, args)),
            Box::new(substitute(b, args)),
            c.as_ref().map(|c| Box::new(substitute(c, args))),
        ),
        Type::Alias(home, name, aargs, aliased) => Type::Alias(
            home.clone(),
            name.clone(),
            aargs
                .iter()
                .map(|(v, t)| (v.clone(), substitute(t, args)))
                .collect(),
            aliased.clone(),
        ),
    }
}

/// `deepDealias`: `ty` with every alias, at any depth, replaced by what
/// it stands for.
fn deep_dealias(ty: &Type) -> Type {
    match ty {
        Type::Alias(_, _, args, aliased) => {
            let args: Vec<(String, Type)> = args
                .iter()
                .map(|(v, t)| (v.clone(), deep_dealias(t)))
                .collect();
            deep_dealias(&dealias(&args, aliased))
        }
        Type::Lambda(a, b) => Type::Lambda(Box::new(deep_dealias(a)), Box::new(deep_dealias(b))),
        Type::Type(home, name, args) => Type::Type(
            home.clone(),
            name.clone(),
            args.iter().map(deep_dealias).collect(),
        ),
        Type::Record(fields, ext) => Type::Record(
            (fields.iter())
                .map(|(n, f)| {
                    let field = crate::canonical::FieldType {
                        order: f.order,
                        ty: deep_dealias(&f.ty),
                    };
                    (n.clone(), field)
                })
                .collect(),
            ext.clone(),
        ),
        Type::Tuple(a, b, c) => Type::Tuple(
            Box::new(deep_dealias(a)),
            Box::new(deep_dealias(b)),
            c.as_ref().map(|c| Box::new(deep_dealias(c))),
        ),
        Type::Var(_) | Type::Unit => ty.clone(),
    }
}

/// `checkPayload`: whether a value of `ty` can cross to JavaScript, or the
/// part that can't, and why.
pub fn check_payload(ty: &Type) -> Result<(), Box<(Type, InvalidPayload)>> {
    match ty {
        Type::Alias(_, _, args, aliased) => check_payload(&dealias(args, aliased)),
        Type::Type(home, name, args) => match &args[..] {
            [] if is(home, "elm/json", "Json.Encode") && name == "Value"
                || is(home, "elm/core", "String") && name == "String"
                || is(home, "elm/core", "Basics")
                    && matches!(name.as_str(), "Int" | "Float" | "Bool") =>
            {
                Ok(())
            }
            [arg]
                if is(home, "elm/core", "List") && name == "List"
                    || is(home, "elm/core", "Maybe") && name == "Maybe"
                    || is(home, "elm/core", "Array") && name == "Array" =>
            {
                check_payload(arg)
            }
            _ => Err(Box::new((
                ty.clone(),
                InvalidPayload::UnsupportedType(name.clone()),
            ))),
        },
        Type::Unit => Ok(()),
        Type::Tuple(a, b, c) => {
            check_payload(a)?;
            check_payload(b)?;
            c.as_ref().map_or(Ok(()), |c| check_payload(c))
        }
        Type::Var(name) => Err(Box::new((
            ty.clone(),
            InvalidPayload::TypeVariable(name.clone()),
        ))),
        Type::Lambda(..) => Err(Box::new((ty.clone(), InvalidPayload::Function))),
        Type::Record(_, Some(_)) => Err(Box::new((ty.clone(), InvalidPayload::ExtendedRecord))),
        Type::Record(fields, None) => fields.values().try_for_each(|f| check_payload(&f.ty)),
    }
}

/// An effect manager's functions and types: `init`, `onEffects` and
/// `onSelfMsg`, then for commands `cmdMap` and their type, for
/// subscriptions `subMap` and theirs.
fn manager(
    module: &hir::Module,
    fields: &[(hir::Spanned, hir::Spanned)],
    unions: &Unions,
) -> R<()> {
    let values: BTreeMap<&str, TextRange> = (module.values.iter())
        .filter_map(|&id| match &module.defs[id] {
            Def::Define { name, .. } => Some((name.text.as_str(), name.range)),
            Def::Destruct { .. } => None,
        })
        .collect();
    // Upstream's region is the header's `where { … }`; its first field
    // stands in, where a report of it needs a place.
    let region = fields.first().map(|(f, _)| f.range).unwrap_or_default();
    let function = |name: &str| match values.get(name) {
        Some(_) => Ok(()),
        None => throw(Error::EffectFunctionNotFound {
            range: region,
            name: name.to_owned(),
        }),
    };
    let effect_type = |name: &hir::Spanned| {
        if unions.contains_key(&name.text) {
            Ok(())
        } else {
            throw(Error::EffectNotFound {
                range: name.range,
                name: name.text.clone(),
            })
        }
    };
    let field = |key: &str| fields.iter().find(|(f, _)| f.text == key).map(|(_, t)| t);
    let kind = match (field("command"), field("subscription")) {
        (Some(cmd), None) => both(effect_type(cmd), function("cmdMap")).map(|_| ()),
        (None, Some(sub)) => both(effect_type(sub), function("subMap")).map(|_| ()),
        (Some(cmd), Some(sub)) => {
            let types = both(effect_type(cmd), effect_type(sub));
            both(both(types, function("cmdMap")), function("subMap")).map(|_| ())
        }
        (None, None) => Ok(()),
    };
    let functions = both(
        both(function("init"), function("onEffects")),
        function("onSelfMsg"),
    );
    both(functions, kind).map(|_| ())
}
