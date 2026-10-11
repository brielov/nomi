//! `Optimize/Port.hs`: the JSON encoders and decoders of ports and of a
//! program's flags, built from their types, which `Effects.checkPayload`
//! has already checked. Record fields are taken in the release's order of
//! names.

use std::collections::BTreeMap;

use nomi_base::Compat;

use super::names::{self, Tracker};
use crate::canonical::{FieldType, Home, Type};
use crate::canonicalize::effects::dealias;
use crate::optimized::{Destructor, Expr, Path};

fn json(module: &str) -> Home {
    Home {
        package: "elm/json".into(),
        module: module.into(),
    }
}

fn maybe() -> Home {
    Home {
        package: "elm/core".into(),
        module: "Maybe".into(),
    }
}

fn encode(t: &mut Tracker, name: &str) -> Expr {
    t.global(&json("Json.Encode"), name)
}

fn decode(t: &mut Tracker, name: &str) -> Expr {
    t.global(&json("Json.Decode"), name)
}

fn call(func: Expr, args: Vec<Expr>) -> Expr {
    Expr::Call(Box::new(func), args)
}

fn local(name: &str) -> Expr {
    Expr::VarLocal(name.to_owned())
}

/// A record's fields in the order upstream lists them (`Map.toList`).
fn fields(compat: Compat, fields: &BTreeMap<String, FieldType>) -> Vec<(&String, &FieldType)> {
    let mut list: Vec<_> = fields.iter().collect();
    if compat == Compat::V0_19_3 {
        list.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(b.0)));
    }
    list
}

/// `toEncoder`.
pub fn to_encoder(compat: Compat, t: &mut Tracker, ty: &Type) -> Expr {
    match ty {
        Type::Alias(_, _, args, alias) => to_encoder(compat, t, &dealias(args, alias)),
        Type::Unit => Expr::Function(vec!["$".to_owned()], Box::new(encode(t, "null"))),
        Type::Tuple(a, b, c) => {
            let list = encode(t, "list");
            let identity = t.global(&names::basics(), "identity");
            let mut encoded = Vec::new();
            for (arg, ty) in [("a", Some(a)), ("b", Some(b)), ("c", c.as_ref())] {
                if let Some(ty) = ty {
                    let encoder = to_encoder(compat, t, ty);
                    encoded.push((arg, call(encoder, vec![local(arg)])));
                }
            }
            let names: Vec<&str> = encoded.iter().map(|(n, _)| *n).collect();
            let body = call(
                list,
                vec![
                    identity,
                    Expr::List(encoded.into_iter().map(|(_, e)| e).collect()),
                ],
            );
            let body = (names.iter().enumerate().rev()).fold(body, |body, (i, name)| {
                let path = Path::Index(i, Box::new(Path::Root("$".to_owned())));
                Expr::Destruct(
                    Destructor {
                        name: (*name).to_owned(),
                        path,
                    },
                    Box::new(body),
                )
            });
            Expr::Function(vec!["$".to_owned()], Box::new(body))
        }
        Type::Type(_, name, args) => match (name.as_str(), args.as_slice()) {
            ("Float", []) => encode(t, "float"),
            ("Int", []) => encode(t, "int"),
            ("Bool", []) => encode(t, "bool"),
            ("String", []) => encode(t, "string"),
            ("Value", []) => t.global(&names::basics(), "identity"),
            ("Maybe", [arg]) => {
                let null = encode(t, "null");
                let encoder = to_encoder(compat, t, arg);
                let destruct = t.global(&maybe(), "destruct");
                let body = call(destruct, vec![null, encoder, local("$")]);
                Expr::Function(vec!["$".to_owned()], Box::new(body))
            }
            ("List", [arg]) => {
                let list = encode(t, "list");
                call(list, vec![to_encoder(compat, t, arg)])
            }
            ("Array", [arg]) => {
                let array = encode(t, "array");
                call(array, vec![to_encoder(compat, t, arg)])
            }
            _ => unreachable!("a port of a type checkPayload refuses"),
        },
        Type::Record(record, None) => {
            let object = encode(t, "object");
            let pairs = (fields(compat, record).into_iter())
                .map(|(name, field)| {
                    let encoder = to_encoder(compat, t, &field.ty);
                    let access = Expr::Access(Box::new(local("$")), name.clone());
                    Expr::Tuple(
                        Box::new(Expr::Str(name.clone())),
                        Box::new(call(encoder, vec![access])),
                        None,
                    )
                })
                .collect();
            for name in record.keys() {
                t.field(name);
            }
            let body = call(object, vec![Expr::List(pairs)]);
            Expr::Function(vec!["$".to_owned()], Box::new(body))
        }
        Type::Lambda(..) | Type::Var(_) | Type::Record(_, Some(_)) => {
            unreachable!("a port of a type checkPayload refuses")
        }
    }
}

/// `toFlagsDecoder`.
pub fn to_flags_decoder(compat: Compat, t: &mut Tracker, ty: &Type) -> Expr {
    match ty {
        Type::Unit => {
            let succeed = decode(t, "succeed");
            call(succeed, vec![Expr::Unit])
        }
        _ => to_decoder(compat, t, ty),
    }
}

/// `toDecoder`.
pub fn to_decoder(compat: Compat, t: &mut Tracker, ty: &Type) -> Expr {
    match ty {
        Type::Alias(_, _, args, alias) => to_decoder(compat, t, &dealias(args, alias)),
        Type::Unit => {
            let null = decode(t, "null");
            call(null, vec![Expr::Unit])
        }
        Type::Tuple(a, b, c) => {
            let succeed = decode(t, "succeed");
            let var = |i: usize| local(&format!("_v{i}"));
            let tuple = Expr::Tuple(
                Box::new(var(0)),
                Box::new(var(1)),
                c.as_ref().map(|_| Box::new(var(2))),
            );
            let mut decoder = call(succeed, vec![tuple]);
            let parts: Vec<&Type> = [Some(&**a), Some(&**b), c.as_deref()]
                .into_iter()
                .flatten()
                .collect();
            for (i, ty) in parts.into_iter().enumerate().rev() {
                decoder = index_and_then(compat, t, i, ty, decoder);
            }
            decoder
        }
        Type::Type(_, name, args) => match (name.as_str(), args.as_slice()) {
            ("Float", []) => decode(t, "float"),
            ("Int", []) => decode(t, "int"),
            ("Bool", []) => decode(t, "bool"),
            ("String", []) => decode(t, "string"),
            ("Value", []) => decode(t, "value"),
            ("Maybe", [arg]) => {
                let nothing = t.global(&maybe(), "Nothing");
                let just = t.global(&maybe(), "Just");
                let one_of = decode(t, "oneOf");
                let null = decode(t, "null");
                let map = decode(t, "map");
                let sub = to_decoder(compat, t, arg);
                call(
                    one_of,
                    vec![Expr::List(vec![
                        call(null, vec![nothing]),
                        call(map, vec![just, sub]),
                    ])],
                )
            }
            ("List", [arg]) => {
                let list = decode(t, "list");
                call(list, vec![to_decoder(compat, t, arg)])
            }
            ("Array", [arg]) => {
                let array = decode(t, "array");
                call(array, vec![to_decoder(compat, t, arg)])
            }
            _ => unreachable!("a port of a type checkPayload refuses"),
        },
        Type::Record(record, None) => {
            let succeed = decode(t, "succeed");
            let value = Expr::Record(record.keys().map(|n| (n.clone(), local(n))).collect());
            for name in record.keys() {
                t.field(name);
            }
            let mut decoder = call(succeed, vec![value]);
            for (key, field) in fields(compat, record) {
                let and_then = decode(t, "andThen");
                let at = decode(t, "field");
                let sub = to_decoder(compat, t, &field.ty);
                decoder = call(
                    and_then,
                    vec![
                        Expr::Function(vec![key.clone()], Box::new(decoder)),
                        call(at, vec![Expr::Str(key.clone()), sub]),
                    ],
                );
            }
            decoder
        }
        Type::Lambda(..) | Type::Var(_) | Type::Record(_, Some(_)) => {
            unreachable!("a port of a type checkPayload refuses")
        }
    }
}

/// `indexAndThen`: the tuple's `i`th element decoded into `_v{i}`, then
/// `decoder`.
fn index_and_then(compat: Compat, t: &mut Tracker, i: usize, ty: &Type, decoder: Expr) -> Expr {
    let and_then = decode(t, "andThen");
    let index = decode(t, "index");
    let sub = to_decoder(compat, t, ty);
    call(
        and_then,
        vec![
            Expr::Function(vec![format!("_v{i}")], Box::new(decoder)),
            call(index, vec![Expr::Int(i as i64), sub]),
        ],
    )
}
