//! The names in scope while canonicalizing a module
//! (`Canonicalize/Environment.hs`): each maps to every home that offers
//! it, so one home resolves and more are ambiguous. Foreign values, types
//! and constructors borrow from the imported interfaces.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use nomi_base::{Compat, TextRange};

use super::error::{Error, NameKind, PossibleNames, R, collected, throw};
use super::references::{Kind, Recorder};
use crate::canonical::{AliasType, Annotation, FieldType, Home, Type, Union, fields_to_list};
use crate::hir::Fixity;

/// Names exposed unqualified: each name's homes (`Env.Exposed`).
pub(super) type Exposed<T> = BTreeMap<String, BTreeMap<Home, T>>;

/// Names under each qualifier (`Env.Qualified`).
pub(super) type Qualified<T> = BTreeMap<String, Exposed<T>>;

/// `Env.Env`. Every scope copies it, so the tables a scope doesn't change
/// are shared.
#[derive(Clone, Debug)]
pub(super) struct Env<'a> {
    /// The release whose order names take where it shows.
    pub(super) compat: Compat,
    pub(super) home: Home,
    pub(super) vars: Rc<BTreeMap<String, Var<'a>>>,
    pub(super) types: Rc<Exposed<TypeInfo>>,
    pub(super) ctors: Rc<Exposed<CtorInfo<'a>>>,
    pub(super) binops: Rc<Exposed<BinopInfo<'a>>>,
    pub(super) q_vars: Rc<Qualified<&'a Annotation>>,
    pub(super) q_types: Rc<Qualified<TypeInfo>>,
    pub(super) q_ctors: Rc<Qualified<CtorInfo<'a>>>,
    /// What each name resolved to, for navigation.
    pub(super) references: Recorder,
}

/// `Env.Var`.
#[derive(Clone, Debug)]
pub(super) enum Var<'a> {
    Local(TextRange),
    TopLevel(TextRange),
    Foreign(BTreeMap<Home, &'a Annotation>),
}

/// `Env.Type`: an alias with its parameters and body, or a union with its
/// arity.
#[derive(Clone, Debug)]
pub(super) enum TypeInfo {
    Alias {
        home: Home,
        vars: Vec<String>,
        ty: Arc<Type>,
    },
    Union {
        arity: usize,
        home: Home,
    },
}

/// `Env.Ctor`: a record alias's constructor, or a union's.
#[derive(Clone, Debug)]
pub(super) enum CtorInfo<'a> {
    Record {
        home: Home,
        vars: Vec<String>,
        ty: Type,
    },
    Ctor {
        home: Home,
        union_name: String,
        /// The union, which its constructors share.
        union: Cow<'a, Arc<Union>>,
        /// Which of the union's constructors, by its place among them.
        ctor: usize,
    },
}

impl CtorInfo<'_> {
    /// A record alias's constructor (`aliasToType`, `addCtors`): a function
    /// from the record's fields, in their written order, to the alias,
    /// which stands for `body`.
    pub(super) fn record(
        compat: Compat,
        home: &Home,
        name: &str,
        vars: &[String],
        fields: &BTreeMap<String, FieldType>,
        body: Arc<Type>,
    ) -> Self {
        let args = vars.iter().map(|v| (v.clone(), Type::Var(v.clone())));
        let result = Type::Alias {
            home: home.clone(),
            name: name.to_owned(),
            args: args.collect(),
            aliased: AliasType::Filled(body),
        };
        let fields = fields_to_list(compat, fields).into_iter();
        CtorInfo::Record {
            home: home.clone(),
            vars: vars.to_vec(),
            ty: Type::function(fields.map(|(_, f)| f.ty.clone()), result),
        }
    }
}

/// `Env.Binop`.
#[derive(Clone, Debug)]
pub(super) struct BinopInfo<'a> {
    pub(super) op: String,
    pub(super) home: Home,
    pub(super) function: String,
    pub(super) annotation: &'a Annotation,
    pub(super) fixity: Fixity,
}

/// Which side keeps the info of a home two tables both have.
#[derive(Clone, Copy)]
pub(super) enum Bias {
    Old,
    New,
}

/// `from`'s homes added to `into`'s (`Map.union`, either way round).
pub(super) fn merge_homes<T: Clone>(
    into: &mut BTreeMap<Home, T>,
    from: &BTreeMap<Home, T>,
    bias: Bias,
) {
    for (home, info) in from {
        match bias {
            Bias::Old => {
                into.entry(home.clone()).or_insert_with(|| info.clone());
            }
            Bias::New => {
                into.insert(home.clone(), info.clone());
            }
        }
    }
}

/// `Map.unionWith Map.union`: each of `from`'s names' homes added to
/// `into`'s.
pub(super) fn merge<T: Clone>(into: &mut Exposed<T>, from: &Exposed<T>, bias: Bias) {
    for (name, homes) in from {
        merge_homes(into.entry(name.clone()).or_default(), homes, bias);
    }
}

/// The one home of the `kind` named `name` and its info, or the error
/// that it is ambiguous among the homes.
pub(super) fn one<'h, T>(
    homes: &'h BTreeMap<Home, T>,
    kind: NameKind,
    range: TextRange,
    prefix: Option<&str>,
    name: &str,
) -> R<(&'h Home, &'h T)> {
    match homes.len() {
        1 => Ok(homes.iter().next().expect("one home")),
        _ => throw(Error::Ambiguous {
            kind,
            range,
            prefix: prefix.map(str::to_owned),
            name: name.to_owned(),
            homes: homes.keys().cloned().collect(),
        }),
    }
}

/// The names a missing one may be a typo of: `locals`, and those under
/// each qualifier.
pub(super) fn possible<'n, T>(
    locals: impl Iterator<Item = &'n String>,
    qualified: &Qualified<T>,
) -> PossibleNames {
    PossibleNames {
        locals: locals.cloned().collect(),
        qualified: (qualified.iter())
            .map(|(prefix, names)| (prefix.clone(), names.keys().cloned().collect()))
            .collect(),
    }
}

/// `findType` and `findCtor`, and their qualified forms, which differ only
/// in which tables they read.
fn find<'e, T>(
    kind: NameKind,
    range: TextRange,
    exposed: &'e Exposed<T>,
    qualified: &'e Qualified<T>,
    prefix: Option<&str>,
    name: &str,
) -> R<&'e T> {
    let table = match prefix {
        None => Some(exposed),
        Some(prefix) => qualified.get(prefix),
    };
    match table.and_then(|t| t.get(name)) {
        Some(homes) => one(homes, kind, range, prefix, name).map(|(_, info)| info),
        None => throw(Error::NotFound {
            kind,
            range,
            prefix: prefix.map(str::to_owned),
            name: name.to_owned(),
            possible: possible(exposed.keys(), qualified),
        }),
    }
}

impl<'a> Env<'a> {
    /// `addLocals`: `names` bound locally over the environment. A local or
    /// top-level name of the same spelling is shadowed, which is an error;
    /// a foreign one just yields.
    pub(super) fn add_locals(&self, names: &BTreeMap<String, TextRange>) -> R<Env<'a>> {
        let mut env = self.clone();
        let mut errors = Vec::new();
        for name in self.compat.sorted_names(names.keys()) {
            let range = names[name];
            match env.vars.get(name) {
                None | Some(Var::Foreign(_)) => {
                    Rc::make_mut(&mut env.vars).insert(name.clone(), Var::Local(range));
                }
                Some(Var::Local(parent) | Var::TopLevel(parent)) => {
                    errors.push(Error::Shadowing {
                        name: name.clone(),
                        first: *parent,
                        second: range,
                    });
                }
            }
        }
        collected(env, errors)
    }

    /// `findType`: the type `name` names, under `prefix` if it has one.
    pub(super) fn find_type(
        &self,
        range: TextRange,
        prefix: Option<&str>,
        name: &str,
    ) -> R<&TypeInfo> {
        let info = find(
            NameKind::Type,
            range,
            &self.types,
            &self.q_types,
            prefix,
            name,
        )?;
        let (TypeInfo::Alias { home, .. } | TypeInfo::Union { home, .. }) = info;
        self.references.global(range, home, name, Kind::Type);
        Ok(info)
    }

    /// `findCtor`: the constructor `name` names, under `prefix` if it has
    /// one.
    pub(super) fn find_ctor(
        &self,
        range: TextRange,
        prefix: Option<&str>,
        name: &str,
    ) -> R<&CtorInfo<'a>> {
        let info = find(
            NameKind::Variant,
            range,
            &self.ctors,
            &self.q_ctors,
            prefix,
            name,
        )?;
        let (CtorInfo::Record { home, .. } | CtorInfo::Ctor { home, .. }) = info;
        self.references.global(range, home, name, Kind::Ctor);
        Ok(info)
    }

    /// `findBinop`: the operator `op`.
    pub(super) fn find_binop(&self, range: TextRange, op: &str) -> R<&BinopInfo<'a>> {
        match self.binops.get(op) {
            Some(homes) => {
                let (_, binop) = one(homes, NameKind::Operator, range, None, op)?;
                self.references.global(range, &binop.home, op, Kind::Binop);
                Ok(binop)
            }
            None => throw(Error::NotFoundBinop {
                range,
                op: op.to_owned(),
                possible: self.binops.keys().cloned().collect(),
            }),
        }
    }
}
