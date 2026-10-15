//! The environment a module's imports make (`Environment/Foreign.hs`,
//! `createInitialEnv`), with the default imports the parser adds
//! (`Elm/Compiler/Imports.hs`). Imports are added in order, and the
//! first that fails stops the rest, as upstream's `foldM` does; so does
//! the first name an import can't expose.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::LazyLock;

use nomi_base::{Compat, TextRange};

use super::env::{
    Bias, BinopInfo, CtorInfo, Env, Exposed, Qualified, TypeInfo, Var, merge, merge_homes,
};
use super::error::{Error, R, throw};
use super::references::{Kind, Recorder};
use crate::canonical::{self, Annotation, Home, Type};
use crate::hir::{Exposed as Item, Exposing, Import, Spanned};
use crate::interface::{self, Interface, Interfaces};

/// The packages whose modules may import kernel code (`Pkg.isKernel`).
pub(super) fn is_kernel_package(package: &str) -> bool {
    package.starts_with("elm/") || package.starts_with("elm-explorations/")
}

/// `Name.isKernel`.
pub(super) fn is_kernel_module(module: &str) -> bool {
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

static DEFAULTS: LazyLock<Vec<Import>> = LazyLock::new(defaults);

/// The imports upstream's parser gives `imports`: the defaults first,
/// except in elm/core.
pub fn with_defaults<'i>(
    package: &str,
    imports: &'i [Import],
) -> impl Iterator<Item = &'i Import> + Clone {
    let defaults: &'static [Import] = if package == "elm/core" {
        &[]
    } else {
        &DEFAULTS
    };
    defaults.iter().chain(imports)
}

#[derive(Default)]
struct State<'a> {
    vars: Exposed<&'a Annotation>,
    types: Exposed<TypeInfo>,
    ctors: Exposed<CtorInfo<'a>>,
    binops: Exposed<BinopInfo<'a>>,
    q_vars: Qualified<&'a Annotation>,
    q_types: Qualified<TypeInfo>,
    q_ctors: Qualified<CtorInfo<'a>>,
}

/// `createInitialEnv`: the environment of a module at `home` whose
/// imports, defaults included, are `imports`, reading each imported
/// module's interface from `interfaces`. An import with no interface was
/// reported by the build already, and adds nothing.
pub(super) fn initial_env<'a, 'i>(
    compat: Compat,
    home: &Home,
    interfaces: &'a dyn Interfaces,
    imports: impl IntoIterator<Item = &'i Import>,
    references: &Recorder,
) -> R<Env<'a>> {
    let mut state = State::default();
    let list = Home::core("List");
    state.types.insert(
        "List".to_owned(),
        BTreeMap::from([(
            list.clone(),
            TypeInfo::Union {
                arity: 1,
                home: list,
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
        add_import(compat, &mut state, interface, import, references)?;
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
struct TypeEntry<'a> {
    info: TypeInfo,
    ctors: Exposed<CtorInfo<'a>>,
}

/// What one import offers, before its exposing list picks from it.
struct Offer<'a> {
    interface: &'a Interface,
    home: Home,
    vars: Exposed<&'a Annotation>,
    types: BTreeMap<String, TypeEntry<'a>>,
}

fn add_import<'a>(
    compat: Compat,
    state: &mut State<'a>,
    interface: &'a Interface,
    import: &Import,
    references: &Recorder,
) -> R<()> {
    let name = &import.name.text;
    let prefix = import.alias.as_ref().unwrap_or(&import.name).text.as_str();
    let home = Home {
        package: interface.home.as_str().into(),
        module: name.into(),
    };
    references.global(import.name.range, &home, name, Kind::Module);
    // Unions before aliases, as `Map.union` keeps the left one.
    let mut types: BTreeMap<String, TypeEntry<'a>> = BTreeMap::new();
    for (type_name, alias) in &interface.aliases {
        if let Some(alias) = alias.public() {
            types.insert(
                type_name.clone(),
                alias_to_type(compat, &home, type_name, alias),
            );
        }
    }
    for (type_name, union) in &interface.unions {
        if let Some(entry) = union_to_type(&home, type_name, union) {
            types.insert(type_name.clone(), entry);
        }
    }
    let vars: Exposed<&'a Annotation> = (interface.values.iter())
        .map(|(n, a)| (n.clone(), single(&home, a)))
        .collect();
    let type_infos: Exposed<TypeInfo> = (types.iter())
        .map(|(n, entry)| (n.clone(), single(&home, entry.info.clone())))
        .collect();
    // A left-biased union folded from the right: the first type's
    // constructors win, so adding each type's in order, keeping what is
    // there, makes the same map without copying it each time.
    let mut ctors: Exposed<CtorInfo<'a>> = BTreeMap::new();
    for entry in types.values() {
        merge(&mut ctors, &entry.ctors, Bias::Old);
    }
    merge(
        state.q_vars.entry(prefix.to_owned()).or_default(),
        &vars,
        Bias::New,
    );
    let q_types = state.q_types.entry(prefix.to_owned()).or_default();
    merge(q_types, &type_infos, Bias::New);
    merge(
        state.q_ctors.entry(prefix.to_owned()).or_default(),
        &ctors,
        Bias::New,
    );
    let offer = Offer {
        interface,
        home,
        vars,
        types,
    };
    match &import.exposing {
        Some(Exposing::Open) => {
            merge(&mut state.vars, &offer.vars, Bias::Old);
            merge(&mut state.types, &type_infos, Bias::Old);
            merge(&mut state.ctors, &ctors, Bias::Old);
            for (op, b) in &interface.binops {
                let homes = state.binops.entry(op.clone()).or_default();
                (homes.entry(offer.home.clone())).or_insert_with(|| offer.binop(op, b));
            }
            Ok(())
        }
        Some(Exposing::Explicit(items)) => items
            .iter()
            .try_for_each(|item| expose(state, &offer, item, references)),
        None => Ok(()),
    }
}

impl<'a> Offer<'a> {
    fn binop(&self, op: &str, b: &'a interface::Binop) -> BinopInfo<'a> {
        BinopInfo {
            op: op.to_owned(),
            home: self.home.clone(),
            function: b.function.clone(),
            annotation: &b.annotation,
            fixity: b.fixity,
        }
    }

    /// `name` isn't among the names of its kind the import offers, which
    /// are `possible`.
    fn not_found<T>(&self, name: &Spanned, possible: Vec<String>) -> R<T> {
        throw(Error::ImportExposingNotFound {
            range: name.range,
            home: self.home.clone(),
            name: name.text.clone(),
            possible,
        })
    }
}

/// One item of an import's exposing list, added unqualified.
fn expose<'a>(
    state: &mut State<'a>,
    offer: &Offer<'a>,
    item: &Item,
    references: &Recorder,
) -> R<()> {
    let home = &offer.home;
    match item {
        Item::Lower(value) => {
            let Some(homes) = offer.vars.get(&value.text) else {
                let possible = offer.vars.keys().cloned().collect();
                return offer.not_found(value, possible);
            };
            references.global(value.range, home, &value.text, Kind::Value);
            let entry = state.vars.entry(value.text.clone()).or_default();
            merge_homes(entry, homes, Bias::New);
        }
        Item::Upper(type_name, open) => match (offer.types.get(&type_name.text), open) {
            (
                Some(TypeEntry {
                    info: TypeInfo::Alias { .. },
                    ..
                }),
                Some(dots),
            ) => {
                return throw(Error::ImportOpenAlias {
                    range: *dots,
                    name: type_name.text.clone(),
                });
            }
            (Some(TypeEntry { info, ctors }), _) => {
                references.global(type_name.range, home, &type_name.text, Kind::Type);
                (state.types).insert(type_name.text.clone(), single(home, info.clone()));
                // A union's constructors come with `(..)`, a record alias's
                // always.
                if open.is_some() || matches!(info, TypeInfo::Alias { .. }) {
                    merge(&mut state.ctors, ctors, Bias::Old);
                }
            }
            (None, open) => {
                if open.is_none()
                    && let Some(union) = union_of_ctor(&offer.types, &type_name.text)
                {
                    return throw(Error::ImportCtorByName {
                        range: type_name.range,
                        ctor: type_name.text.clone(),
                        union,
                    });
                }
                let possible = offer.types.keys().cloned().collect();
                return offer.not_found(type_name, possible);
            }
        },
        Item::Operator(op) => {
            let Some(b) = offer.interface.binops.get(&op.text) else {
                let possible = offer.interface.binops.keys().cloned().collect();
                return offer.not_found(op, possible);
            };
            references.global(op.range, home, &op.text, Kind::Binop);
            (state.binops).insert(op.text.clone(), single(home, offer.binop(&op.text, b)));
        }
    }
    Ok(())
}

/// One home's info.
fn single<T>(home: &Home, info: T) -> BTreeMap<Home, T> {
    BTreeMap::from([(home.clone(), info)])
}

/// `toCtors`: the union a constructor of `types` belongs to.
fn union_of_ctor(types: &BTreeMap<String, TypeEntry<'_>>, ctor: &str) -> Option<String> {
    types
        .values()
        .find_map(|entry| match entry.ctors.get(ctor) {
            Some(homes) if homes.len() == 1 => match homes.values().next() {
                Some(CtorInfo::Ctor { union_name, .. }) => Some(union_name.clone()),
                Some(CtorInfo::Record { .. }) | None => None,
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
    let (union, ctors) = match union {
        interface::Union::Open(u) => (u, &u.ctors[..]),
        interface::Union::Closed(u) => (u, &[][..]),
        interface::Union::Private(_) => return None,
    };
    let ctors = (ctors.iter().enumerate())
        .map(|(at, ctor)| {
            let info = CtorInfo::Ctor {
                home: home.clone(),
                union_name: name.to_owned(),
                union: Cow::Borrowed(union),
                ctor: at,
            };
            (ctor.name.clone(), single(home, info))
        })
        .collect();
    let info = TypeInfo::Union {
        arity: union.vars.len(),
        home: home.clone(),
    };
    Some(TypeEntry { info, ctors })
}

/// `aliasToType`: the alias, and the constructor a record alias brings.
fn alias_to_type<'a>(
    compat: Compat,
    home: &Home,
    name: &str,
    alias: &canonical::Alias,
) -> TypeEntry<'a> {
    let info = TypeInfo::Alias {
        home: home.clone(),
        vars: alias.vars.clone(),
        ty: alias.ty.clone(),
    };
    let mut ctors = BTreeMap::new();
    if let Type::Record(fields, None) = &*alias.ty {
        let ctor = CtorInfo::record(compat, home, name, &alias.vars, fields, alias.ty.clone());
        ctors.insert(name.to_owned(), single(home, ctor));
    }
    TypeEntry { info, ctors }
}
