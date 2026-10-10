//! The environment a module's imports make (`Environment/Foreign.hs`,
//! `createInitialEnv`), with the default imports the parser adds
//! (`Elm/Compiler/Imports.hs`). Imports are added in order, and the
//! first that fails stops the rest, as upstream's `foldM` does; so does
//! the first name an import can't expose.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use nomi_base::{Compat, TextRange};

use super::env::{BinopInfo, CtorInfo, Env, Exposed, TypeInfo, Var, add_exposed};
use super::error::{Error, NameKind, R, throw};
use super::references::{Kind, Recorder};
use crate::canonical::{AliasType, Annotation, Home, Type};
use crate::hir::{Exposed as Item, Exposing, Import, Spanned};
use crate::interface::{self, Interface};

/// The packages whose modules may import kernel code (`Pkg.isKernel`).
pub fn is_kernel_package(package: &str) -> bool {
    package.starts_with("elm/") || package.starts_with("elm-explorations/")
}

/// `Name.isKernel`.
pub fn is_kernel_module(module: &str) -> bool {
    module.starts_with("Elm.Kernel.")
}

fn spanned(text: &str) -> Spanned {
    Spanned {
        text: text.to_owned(),
        range: TextRange::default(),
    }
}

fn default_import(module: &str, alias: Option<&str>, exposing: Vec<Item>) -> Import {
    Import {
        name: spanned(module),
        alias: alias.map(spanned),
        exposing: Some(Exposing::Explicit(exposing)),
    }
}

/// `Imports.defaults`, in the order the parser leaves them: reversed,
/// since it conses each import onto them and reverses the whole list.
fn defaults() -> Vec<Import> {
    let closed_type = |name: &str| vec![Item::Upper(spanned(name), None)];
    let open_type = |name: &str| vec![Item::Upper(spanned(name), Some(TextRange::default()))];
    let mut defaults = vec![
        Import {
            name: spanned("Basics"),
            alias: None,
            exposing: Some(Exposing::Open),
        },
        default_import("Debug", None, vec![]),
        default_import("List", None, vec![Item::Operator(spanned("::"))]),
        default_import("Maybe", None, open_type("Maybe")),
        default_import("Result", None, open_type("Result")),
        default_import("String", None, closed_type("String")),
        default_import("Char", None, closed_type("Char")),
        default_import("Tuple", None, vec![]),
        default_import("Platform", None, closed_type("Program")),
        default_import("Platform.Cmd", Some("Cmd"), closed_type("Cmd")),
        default_import("Platform.Sub", Some("Sub"), closed_type("Sub")),
    ];
    defaults.reverse();
    defaults
}

/// The imports upstream's parser gives `imports`: the defaults first,
/// except in elm/core.
pub fn with_defaults(package: &str, imports: &[Import]) -> Vec<Import> {
    let mut all = if package == "elm/core" {
        Vec::new()
    } else {
        defaults()
    };
    all.extend(imports.iter().cloned());
    all
}

#[derive(Default)]
struct State<'a> {
    vars: Exposed<&'a Annotation>,
    types: Exposed<TypeInfo<'a>>,
    ctors: Exposed<CtorInfo<'a>>,
    binops: Exposed<BinopInfo<'a>>,
    q_vars: BTreeMap<String, Exposed<&'a Annotation>>,
    q_types: BTreeMap<String, Exposed<TypeInfo<'a>>>,
    q_ctors: BTreeMap<String, Exposed<CtorInfo<'a>>>,
}

/// `createInitialEnv`: the environment of a module at `home` whose
/// imports, defaults included, are `imports`, reading each imported
/// module's interface from `interfaces`. An import with no interface was
/// reported by the build already, and adds nothing.
pub fn initial_env<'a>(
    compat: Compat,
    home: &Home,
    interfaces: &'a HashMap<String, Interface>,
    imports: &[Import],
    references: &Recorder,
) -> R<Env<'a>> {
    let core = |module: &str| Home {
        package: "elm/core".to_owned(),
        module: module.to_owned(),
    };
    let mut state = State::default();
    state.types.insert(
        "List".to_owned(),
        BTreeMap::from([(
            core("List"),
            TypeInfo::Union {
                arity: 1,
                home: core("List"),
            },
        )]),
    );
    let kernel_package = is_kernel_package(&home.package);
    for import in imports {
        // `toSafeImports`: a kernel package's kernel imports aren't modules.
        if kernel_package && is_kernel_module(&import.name.text) {
            continue;
        }
        let Some(interface) = interfaces.get(&import.name.text) else {
            continue;
        };
        add_import(&mut state, interface, import, references)?;
    }
    Ok(Env {
        compat,
        home: home.clone(),
        vars: Rc::new(
            (state.vars.into_iter())
                .map(|(name, homes)| (name, Var::Foreign(homes)))
                .collect(),
        ),
        types: Rc::new(state.types),
        ctors: Rc::new(state.ctors),
        binops: Rc::new(state.binops),
        q_vars: Rc::new(state.q_vars),
        q_types: Rc::new(state.q_types),
        q_ctors: Rc::new(state.q_ctors),
        references: references.clone(),
    })
}

/// A type an import offers, and the constructors it brings.
type TypeEntry<'a> = (TypeInfo<'a>, Exposed<CtorInfo<'a>>);

fn add_import<'a>(
    state: &mut State<'a>,
    interface: &'a Interface,
    import: &Import,
    references: &Recorder,
) -> R<()> {
    let name = &import.name.text;
    let prefix = import.alias.as_ref().map_or(name, |a| &a.text).clone();
    let home = Home {
        package: interface.home.clone(),
        module: name.clone(),
    };
    references.global(import.name.range, &home, name, Kind::Module);
    // Unions before aliases, as `Map.union` keeps the left one.
    let mut raw: BTreeMap<String, TypeEntry<'a>> = BTreeMap::new();
    for (type_name, alias) in &interface.aliases {
        if let Some(alias) = alias.public() {
            raw.insert(type_name.clone(), alias_to_type(&home, type_name, alias));
        }
    }
    for (type_name, union) in &interface.unions {
        if let Some(entry) = union_to_type(&home, type_name, union) {
            raw.insert(type_name.clone(), entry);
        }
    }
    let vars: Exposed<&'a Annotation> = (interface.values.iter())
        .map(|(n, a)| (n.clone(), single(&home, a)))
        .collect();
    let types: Exposed<TypeInfo<'a>> = (raw.iter())
        .map(|(n, (t, _))| (n.clone(), single(&home, t.clone())))
        .collect();
    let mut ctors: Exposed<CtorInfo<'a>> = BTreeMap::new();
    for (_, type_ctors) in raw.values().rev() {
        let mut merged = type_ctors.clone();
        add_exposed(&mut merged, &ctors);
        ctors = merged;
    }
    add_qualified(&mut state.q_vars, &prefix, &vars);
    add_qualified(&mut state.q_types, &prefix, &types);
    add_qualified(&mut state.q_ctors, &prefix, &ctors);
    let binop = |op: &str, b: &'a interface::Binop| BinopInfo {
        op: op.to_owned(),
        home: home.clone(),
        function: b.function.clone(),
        annotation: &b.annotation,
        associativity: b.associativity,
        precedence: b.precedence,
    };
    match import
        .exposing
        .as_ref()
        .unwrap_or(&Exposing::Explicit(Vec::new()))
    {
        Exposing::Open => {
            add_exposed(&mut state.vars, &vars);
            add_exposed(&mut state.types, &types);
            add_exposed(&mut state.ctors, &ctors);
            let binops: Exposed<BinopInfo<'a>> = (interface.binops.iter())
                .map(|(op, b)| (op.clone(), single(&home, binop(op, b))))
                .collect();
            add_exposed(&mut state.binops, &binops);
        }
        Exposing::Explicit(items) => {
            for item in items {
                match item {
                    Item::Lower(value) => match vars.get(&value.text) {
                        Some(homes) => {
                            references.global(value.range, &home, &value.text, Kind::Value);
                            let entry = state.vars.entry(value.text.clone()).or_default();
                            for (h, a) in homes {
                                entry.insert(h.clone(), a);
                            }
                        }
                        None => {
                            return throw(Error::ImportExposingNotFound {
                                range: value.range,
                                kind: NameKind::Value,
                                home,
                                name: value.text.clone(),
                                possible: vars.keys().cloned().collect(),
                            });
                        }
                    },
                    Item::Upper(type_name, open) => match (raw.get(&type_name.text), open) {
                        (Some((info @ TypeInfo::Union { .. }, type_ctors)), _) => {
                            references.global(type_name.range, &home, &type_name.text, Kind::Type);
                            state
                                .types
                                .insert(type_name.text.clone(), single(&home, info.clone()));
                            if open.is_some() {
                                add_exposed(&mut state.ctors, type_ctors);
                            }
                        }
                        (Some((info @ TypeInfo::Alias { .. }, type_ctors)), None) => {
                            references.global(type_name.range, &home, &type_name.text, Kind::Type);
                            state
                                .types
                                .insert(type_name.text.clone(), single(&home, info.clone()));
                            add_exposed(&mut state.ctors, type_ctors);
                        }
                        (Some((TypeInfo::Alias { .. }, _)), Some(dots)) => {
                            return throw(Error::ImportOpenAlias {
                                range: *dots,
                                name: type_name.text.clone(),
                            });
                        }
                        (None, open) => {
                            let ctor_of = (open.is_none())
                                .then(|| union_of_ctor(&raw, &type_name.text))
                                .flatten();
                            return throw(match ctor_of {
                                Some(union) => Error::ImportCtorByName {
                                    range: type_name.range,
                                    ctor: type_name.text.clone(),
                                    union,
                                },
                                None => Error::ImportExposingNotFound {
                                    range: type_name.range,
                                    kind: NameKind::Type,
                                    home,
                                    name: type_name.text.clone(),
                                    possible: raw.keys().cloned().collect(),
                                },
                            });
                        }
                    },
                    Item::Operator(op) => match interface.binops.get(&op.text) {
                        Some(b) => {
                            references.global(op.range, &home, &op.text, Kind::Binop);
                            state
                                .binops
                                .insert(op.text.clone(), single(&home, binop(&op.text, b)));
                        }
                        None => {
                            return throw(Error::ImportExposingNotFound {
                                range: op.range,
                                kind: NameKind::Operator,
                                home,
                                name: op.text.clone(),
                                possible: interface.binops.keys().cloned().collect(),
                            });
                        }
                    },
                }
            }
        }
    }
    Ok(())
}

/// One home's info.
fn single<T>(home: &Home, info: T) -> BTreeMap<Home, T> {
    BTreeMap::from([(home.clone(), info)])
}

/// `Map.insertWith addExposed prefix exposed`.
fn add_qualified<T: Clone>(
    qualified: &mut BTreeMap<String, Exposed<T>>,
    prefix: &str,
    exposed: &Exposed<T>,
) {
    match qualified.get_mut(prefix) {
        None => {
            qualified.insert(prefix.to_owned(), exposed.clone());
        }
        Some(old) => {
            let mut merged = exposed.clone();
            add_exposed(&mut merged, old);
            *old = merged;
        }
    }
}

/// `toCtors`: the union a constructor of `raw` belongs to.
fn union_of_ctor(raw: &BTreeMap<String, TypeEntry<'_>>, ctor: &str) -> Option<String> {
    raw.values().find_map(|(_, ctors)| match ctors.get(ctor) {
        Some(homes) if homes.len() == 1 => match homes.values().next() {
            Some(CtorInfo::Ctor { union_name, .. }) => Some(union_name.clone()),
            _ => None,
        },
        _ => None,
    })
}

/// `unionToType`: the union as importers see it, and its constructors.
fn union_to_type<'a>(
    home: &Home,
    name: &str,
    union: &'a interface::Union,
) -> Option<TypeEntry<'a>> {
    let public = match union {
        interface::Union::Open(u) => Cow::Borrowed(u),
        interface::Union::Closed(_) => Cow::Owned(union.public()?),
        interface::Union::Private(_) => return None,
    };
    let mut ctors: Exposed<CtorInfo<'a>> = BTreeMap::new();
    if let Cow::Borrowed(u) = &public {
        for ctor in &u.ctors {
            let info = CtorInfo::Ctor {
                home: home.clone(),
                union_name: name.to_owned(),
                union: Cow::Borrowed(*u),
                index: ctor.index,
                args: Cow::Borrowed(&ctor.args),
            };
            ctors.insert(ctor.name.clone(), BTreeMap::from([(home.clone(), info)]));
        }
    }
    let info = TypeInfo::Union {
        arity: public.vars.len(),
        home: home.clone(),
    };
    Some((info, ctors))
}

/// `aliasToType`: the alias, and the constructor a record alias brings.
fn alias_to_type<'a>(home: &Home, name: &str, alias: &'a crate::canonical::Alias) -> TypeEntry<'a> {
    let info = TypeInfo::Alias {
        arity: alias.vars.len(),
        home: home.clone(),
        vars: alias.vars.clone(),
        ty: Cow::Borrowed(&alias.ty),
    };
    let mut ctors = BTreeMap::new();
    if let Type::Record(fields, None) = &alias.ty {
        let args = alias
            .vars
            .iter()
            .map(|v| (v.clone(), Type::Var(v.clone())))
            .collect();
        let result = Type::Alias(
            home.clone(),
            name.to_owned(),
            args,
            AliasType::Filled(Box::new(alias.ty.clone())),
        );
        let mut by_order: Vec<_> = fields.iter().collect();
        by_order.sort_by_key(|(_, f)| f.order);
        let ty = by_order.into_iter().rev().fold(result, |acc, (_, f)| {
            Type::Lambda(Box::new(f.ty.clone()), Box::new(acc))
        });
        let info = CtorInfo::Record {
            home: home.clone(),
            vars: alias.vars.clone(),
            ty,
        };
        ctors.insert(name.to_owned(), BTreeMap::from([(home.clone(), info)]));
    }
    (info, ctors)
}
