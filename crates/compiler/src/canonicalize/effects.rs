//! Ports and effect managers (`Canonicalize/Effects.hs`): a port must be
//! a command sending, or a subscription receiving, a value that can cross
//! to JavaScript; an effect manager must define the functions and types
//! its header names.

use std::collections::BTreeMap;

use nomi_base::{TextRange, grow};

use super::env::Env;
use super::error::{Error, InvalidPayload, PortProblem, R, all, both, throw};
use super::local::Unions;
use super::types;
use crate::canonical::{Effects, Manager, Port as CPort, PortDirection, Type};
use crate::hir::{self, ManagerFields, ModuleKind};

/// `canonicalize`: the module's ports, in upstream's reverse order, or
/// its effect manager.
pub(super) fn canonicalize(env: &Env<'_>, module: &hir::Module, unions: &Unions) -> R<Effects> {
    match &module.kind {
        ModuleKind::Plain => Ok(Effects::NoEffects),
        ModuleKind::Port => all(module.ports.iter().rev().map(|p| port(env, module, p)))
            .map(|ports| Effects::Ports(ports.into_iter().collect())),
        ModuleKind::Effect(fields) => manager(module, fields, unions),
    }
}

/// `canonicalizePort`.
fn port(env: &Env<'_>, module: &hir::Module, port: &hir::Port) -> R<(String, CPort)> {
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
    let mut parts = delambda(annotation.ty.deep_dealias());
    let Some(Type::Type {
        home,
        name: type_name,
        args,
    }) = parts.pop()
    else {
        return invalid(PortProblem::NotCmdOrSub);
    };
    let [msg] = &args[..] else {
        return invalid(PortProblem::NotCmdOrSub);
    };
    let payload = |ty: &Type, direction| match check_payload(ty) {
        Ok(()) => Ok((
            name.clone(),
            CPort {
                direction,
                free: annotation.free,
                payload: ty.clone(),
                func: annotation.ty,
            },
        )),
        Err(problem) => throw(Error::PortPayloadInvalid {
            range: region,
            name: name.clone(),
            problem,
        }),
    };
    if home.is("elm/core", "Platform.Cmd") && type_name == "Cmd" {
        match &parts[..] {
            [] => invalid(PortProblem::CmdNoArg),
            [outgoing] => match msg {
                Type::Var(_) => payload(outgoing, PortDirection::Outgoing),
                _ => invalid(PortProblem::CmdBadMsg),
            },
            _ => invalid(PortProblem::CmdExtraArgs(parts.len())),
        }
    } else if home.is("elm/core", "Platform.Sub") && type_name == "Sub" {
        match (&parts[..], msg) {
            ([Type::Lambda(incoming, result)], Type::Var(msg2)) if matches!(&**result, Type::Var(msg1) if msg1 == msg2) => {
                payload(incoming, PortDirection::Incoming)
            }
            _ => invalid(PortProblem::SubBad),
        }
    } else {
        invalid(PortProblem::NotCmdOrSub)
    }
}

/// `delambda`: a function's arguments, then its result.
fn delambda(mut ty: Type) -> Vec<Type> {
    let mut parts = Vec::new();
    while let Type::Lambda(arg, result) = ty {
        parts.push(*arg);
        ty = *result;
    }
    parts.push(ty);
    parts
}

/// `checkPayload`: whether a value of `ty` can cross to JavaScript, or why
/// a part of it can't.
pub(crate) fn check_payload(ty: &Type) -> Result<(), InvalidPayload> {
    grow(|| match ty {
        Type::Alias { args, aliased, .. } => check_payload(&aliased.dealias(args)),
        Type::Type { home, name, args } => match &args[..] {
            [] if home.is("elm/json", "Json.Encode") && name == "Value"
                || home.is("elm/core", "String") && name == "String"
                || home.is("elm/core", "Basics")
                    && matches!(name.as_str(), "Int" | "Float" | "Bool") =>
            {
                Ok(())
            }
            [arg]
                if home.is("elm/core", "List") && name == "List"
                    || home.is("elm/core", "Maybe") && name == "Maybe"
                    || home.is("elm/core", "Array") && name == "Array" =>
            {
                check_payload(arg)
            }
            _ => Err(InvalidPayload::UnsupportedType(name.clone())),
        },
        Type::Unit => Ok(()),
        Type::Tuple(a, b, c) => {
            check_payload(a)?;
            check_payload(b)?;
            c.as_ref().map_or(Ok(()), |c| check_payload(c))
        }
        Type::Var(name) => Err(InvalidPayload::TypeVariable(name.clone())),
        Type::Lambda(..) => Err(InvalidPayload::Function),
        Type::Record(_, Some(_)) => Err(InvalidPayload::ExtendedRecord),
        Type::Record(fields, None) => fields.values().try_for_each(|f| check_payload(&f.ty)),
    })
}

/// An effect manager's functions and types: `init`, `onEffects` and
/// `onSelfMsg`, then for commands `cmdMap` and their type, for
/// subscriptions `subMap` and theirs.
fn manager(module: &hir::Module, fields: &ManagerFields, unions: &Unions) -> R<Effects> {
    let values: BTreeMap<&str, TextRange> = (module.value_names())
        .map(|name| (name.text.as_str(), name.range))
        .collect();
    // Upstream's region is the header's `where { … }`; its first field
    // stands in, where a report of it needs a place.
    let function = |name: &str| match values.get(name) {
        Some(&range) => Ok(range),
        None => throw(Error::EffectFunctionNotFound {
            range: fields.first,
            name: name.to_owned(),
        }),
    };
    let effect_type = |name: &hir::Spanned| {
        if unions.contains_key(&name.text) {
            Ok(name.text.clone())
        } else {
            throw(Error::EffectNotFound {
                range: name.range,
                name: name.text.clone(),
            })
        }
    };
    let kind = match (&fields.command, &fields.subscription) {
        (Some(cmd), None) => {
            both(effect_type(cmd), function("cmdMap")).map(|(c, _)| Some(Manager::Cmd(c)))
        }
        (None, Some(sub)) => {
            both(effect_type(sub), function("subMap")).map(|(s, _)| Some(Manager::Sub(s)))
        }
        (Some(cmd), Some(sub)) => {
            let types = both(effect_type(cmd), effect_type(sub));
            both(both(types, function("cmdMap")), function("subMap"))
                .map(|(((cmd, sub), _), _)| Some(Manager::Fx { cmd, sub }))
        }
        // The grammar reads a manager with neither; elm's parser doesn't.
        (None, None) => Ok(None),
    };
    let functions = both(
        both(function("init"), function("onEffects")),
        function("onSelfMsg"),
    );
    both(functions, kind).map(|(((init, on_effects), on_self_msg), kind)| match kind {
        Some(manager) => Effects::Manager {
            init,
            on_effects,
            on_self_msg,
            manager,
        },
        None => Effects::NoEffects,
    })
}
