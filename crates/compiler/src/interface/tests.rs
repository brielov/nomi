use std::collections::BTreeMap;

use proptest::prelude::*;

use super::*;
use crate::canonical::{AliasType, Ctor, CtorOpts, FieldType, Home, Type};
use crate::hir::Associativity;

fn maybe(a: Type) -> Type {
    Type::Type {
        home: Home::core("Maybe"),
        name: "Maybe".to_owned(),
        args: vec![a],
    }
}

/// Part of `Maybe`'s and `Basics`' interfaces.
fn sample() -> Interface {
    let a = || Type::Var("a".to_owned());
    let union = canonical::Union {
        vars: vec!["a".to_owned()],
        ctors: vec![
            Ctor {
                name: "Just".to_owned(),
                index: 0,
                args: vec![a()],
            },
            Ctor {
                name: "Nothing".to_owned(),
                index: 1,
                args: vec![],
            },
        ],
        count: 2,
        opts: CtorOpts::Normal,
    };
    let point = Type::Record(
        BTreeMap::from([
            (
                "x".to_owned(),
                FieldType {
                    order: 0,
                    ty: Type::Unit,
                },
            ),
            (
                "y".to_owned(),
                FieldType {
                    order: 1,
                    ty: maybe(a()),
                },
            ),
        ]),
        Some("r".to_owned()),
    );
    let aliased = Type::Alias {
        home: Home::core("Maybe"),
        name: "Point".to_owned(),
        args: vec![("r".to_owned(), Type::Unit)],
        aliased: AliasType::Filled(std::sync::Arc::new(point.clone())),
    };
    let with_default = Type::Lambda(
        Box::new(a()),
        Box::new(Type::Lambda(Box::new(maybe(a())), Box::new(a()))),
    );
    Interface {
        home: "elm/core".to_owned(),
        values: BTreeMap::from([(
            "withDefault".to_owned(),
            Annotation {
                free: vec!["a".to_owned()],
                ty: with_default.into(),
            },
        )]),
        unions: BTreeMap::from([("Maybe".to_owned(), Union::Open(union))]),
        aliases: BTreeMap::from([(
            "Point".to_owned(),
            Alias::Private(canonical::Alias {
                vars: vec!["r".to_owned()],
                ty: point.into(),
            }),
        )]),
        binops: BTreeMap::from([(
            "|>".to_owned(),
            Binop {
                function: "apR".to_owned(),
                annotation: Annotation {
                    free: vec![],
                    ty: Type::Tuple(Box::new(aliased), Box::new(Type::Unit), None).into(),
                },
                fixity: Fixity {
                    associativity: Associativity::Left,
                    precedence: 0,
                },
            },
        )]),
    }
}

#[test]
fn interfaces_round_trip() {
    let interface = sample();
    let bytes = interface.encode();
    assert_eq!(Interface::decode(&bytes), Some(interface.clone()));
    // The same interface, built again, encodes alike; another doesn't.
    assert_eq!(sample().encode(), bytes);
    let mut other = sample();
    other.binops.get_mut("|>").unwrap().fixity.precedence = 1;
    assert_ne!(other.encode(), bytes);
    for dependency in [
        DependencyInterface::Public(interface.clone()),
        DependencyInterface::Public(interface).privatize(),
    ] {
        assert_eq!(
            DependencyInterface::decode(&dependency.encode()),
            Some(dependency)
        );
    }
}

#[test]
fn damaged_bytes_are_none() {
    let bytes = sample().encode();
    for cut in 0..bytes.len() {
        assert_eq!(Interface::decode(&bytes[..cut]), None, "cut at {cut}");
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert_eq!(Interface::decode(&longer), None);
    let mut newer = bytes;
    newer[0] += 1;
    assert_eq!(Interface::decode(&newer), None);
    // A length past the end of the bytes is no allocation.
    let mut huge = vec![1];
    huge.extend_from_slice(&u64::MAX.to_le_bytes());
    assert_eq!(Interface::decode(&huge), None);
}

#[test]
fn importers_see_the_public_parts() {
    let interface = sample();
    assert_eq!(interface.aliases["Point"].public(), None);
    let DependencyInterface::Private {
        unions, aliases, ..
    } = DependencyInterface::Public(interface).privatize()
    else {
        panic!("privatize kept a public interface");
    };
    assert_eq!(unions["Maybe"].ctors.len(), 2);
    assert!(aliases.contains_key("Point"));
}

fn name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_]{0,4}"
}

fn ty() -> impl Strategy<Value = Type> {
    let leaf = prop_oneof![name().prop_map(Type::Var), Just(Type::Unit)];
    leaf.prop_recursive(4, 24, 3, |inner| {
        let fields = prop::collection::btree_map(name(), (any::<u16>(), inner.clone()), 0..3);
        prop_oneof![
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| Type::Lambda(Box::new(a), Box::new(b))),
            (name(), name(), prop::collection::vec(inner.clone(), 0..3)).prop_map(
                |(m, name, args)| Type::Type {
                    home: Home::core(&m),
                    name,
                    args
                }
            ),
            (fields, prop::option::of(name())).prop_map(|(fields, ext)| {
                let fields = fields
                    .into_iter()
                    .map(|(k, (order, ty))| (k, FieldType { order, ty }))
                    .collect();
                Type::Record(fields, ext)
            }),
            (
                inner.clone(),
                inner.clone(),
                prop::option::of(inner.clone())
            )
                .prop_map(|(a, b, c)| Type::Tuple(
                    Box::new(a),
                    Box::new(b),
                    c.map(Box::new)
                )),
            (
                name(),
                prop::collection::vec((name(), inner.clone()), 0..2),
                inner,
                any::<bool>()
            )
                .prop_map(|(n, args, body, filled)| {
                    let body = Box::new(body);
                    let aliased = if filled {
                        AliasType::Filled(body.into())
                    } else {
                        AliasType::Holey(body.into())
                    };
                    Type::Alias {
                        home: Home::core("M"),
                        name: n,
                        args,
                        aliased,
                    }
                }),
        ]
    })
}

fn annotation() -> impl Strategy<Value = Annotation> {
    (prop::collection::vec(name(), 0..3), ty()).prop_map(|(free, ty)| Annotation {
        free,
        ty: ty.into(),
    })
}

fn interface() -> impl Strategy<Value = Interface> {
    let ctors = prop::collection::vec((name(), prop::collection::vec(ty(), 0..2)), 0..3);
    let union =
        (prop::collection::vec(name(), 0..2), ctors, 0..3u8).prop_map(|(vars, ctors, tag)| {
            let count = ctors.len();
            let ctors = ctors
                .into_iter()
                .enumerate()
                .map(|(index, (name, args))| Ctor { name, index, args })
                .collect();
            let union = canonical::Union {
                vars,
                ctors,
                count,
                opts: CtorOpts::Normal,
            };
            match tag {
                0 => Union::Open(union),
                1 => Union::Closed(union),
                _ => Union::Private(union),
            }
        });
    let alias = (prop::collection::vec(name(), 0..2), ty(), any::<bool>()).prop_map(
        |(vars, ty, public)| {
            let alias = canonical::Alias {
                vars,
                ty: ty.into(),
            };
            if public {
                Alias::Public(alias)
            } else {
                Alias::Private(alias)
            }
        },
    );
    let binop = (name(), annotation(), 0..3usize, any::<i64>()).prop_map(
        |(function, annotation, a, precedence)| Binop {
            function,
            annotation,
            fixity: Fixity {
                associativity: [
                    Associativity::Left,
                    Associativity::Non,
                    Associativity::Right,
                ][a],
                precedence,
            },
        },
    );
    (
        name(),
        prop::collection::btree_map(name(), annotation(), 0..3),
        prop::collection::btree_map(name(), union, 0..3),
        prop::collection::btree_map(name(), alias, 0..3),
        prop::collection::btree_map(name(), binop, 0..2),
    )
        .prop_map(|(home, values, unions, aliases, binops)| Interface {
            home,
            values,
            unions,
            aliases,
            binops,
        })
}

proptest! {
    #[test]
    fn any_interface_round_trips(interface in interface()) {
        prop_assert_eq!(Interface::decode(&interface.encode()), Some(interface));
    }
}
