//! The names in scope while canonicalizing a module
//! (`Canonicalize/Environment.hs`): each maps to every home that offers
//! it, so one home resolves and more are ambiguous. Foreign values, types
//! and constructors borrow from the imported interfaces.

use std::borrow::Cow;
use std::collections::BTreeMap;

use nomi_base::{Compat, TextRange};

use super::error::{Error, NameKind, PossibleNames, R, throw};
use crate::canonical::{Annotation, Associativity, Home, Type, Union};

/// Names exposed unqualified: each name's homes (`Env.Exposed`).
pub type Exposed<T> = BTreeMap<String, BTreeMap<Home, T>>;

/// Names under each qualifier (`Env.Qualified`).
pub type Qualified<T> = BTreeMap<String, Exposed<T>>;

/// `Env.Env`.
#[derive(Clone, Debug)]
pub struct Env<'a> {
    /// The release whose order names take where it shows.
    pub compat: Compat,
    pub home: Home,
    pub vars: BTreeMap<String, Var<'a>>,
    pub types: Exposed<TypeInfo<'a>>,
    pub ctors: Exposed<CtorInfo<'a>>,
    pub binops: Exposed<BinopInfo<'a>>,
    pub q_vars: Qualified<&'a Annotation>,
    pub q_types: Qualified<TypeInfo<'a>>,
    pub q_ctors: Qualified<CtorInfo<'a>>,
}

/// `Env.Var`.
#[derive(Clone, Debug)]
pub enum Var<'a> {
    Local(TextRange),
    TopLevel(TextRange),
    Foreign(BTreeMap<Home, &'a Annotation>),
}

/// `Env.Type`: an alias with its arity, parameters and body, or a union
/// with its arity.
#[derive(Clone, Debug)]
pub enum TypeInfo<'a> {
    Alias {
        arity: usize,
        home: Home,
        vars: Vec<String>,
        ty: Cow<'a, Type>,
    },
    Union {
        arity: usize,
        home: Home,
    },
}

/// `Env.Ctor`: a record alias's constructor, or a union's.
#[derive(Clone, Debug)]
pub enum CtorInfo<'a> {
    Record {
        home: Home,
        vars: Vec<String>,
        ty: Type,
    },
    Ctor {
        home: Home,
        union_name: String,
        union: Cow<'a, Union>,
        index: usize,
        args: Cow<'a, [Type]>,
    },
}

/// `Env.Binop`.
#[derive(Clone, Debug)]
pub struct BinopInfo<'a> {
    pub op: String,
    pub home: Home,
    pub function: String,
    pub annotation: &'a Annotation,
    pub associativity: Associativity,
    pub precedence: i64,
}

/// `Map.unionWith Map.union`: `b`'s homes added to `a`'s, `a`'s kept on
/// a clash.
pub fn add_exposed<T: Clone>(a: &mut Exposed<T>, b: &Exposed<T>) {
    for (name, homes) in b {
        let entry = a.entry(name.clone()).or_default();
        for (home, info) in homes {
            entry.entry(home.clone()).or_insert_with(|| info.clone());
        }
    }
}

/// The one home's info, or an ambiguity.
fn one<T>(homes: &BTreeMap<Home, T>) -> Result<&T, Vec<Home>> {
    match homes.len() {
        1 => Ok(homes.values().next().expect("one home")),
        _ => Err(homes.keys().cloned().collect()),
    }
}

fn possible<T>(exposed: &Exposed<T>, qualified: &Qualified<T>) -> PossibleNames {
    PossibleNames {
        locals: exposed.keys().cloned().collect(),
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
    let not_found = || Error::NotFound {
        kind,
        range,
        prefix: prefix.map(str::to_owned),
        name: name.to_owned(),
        possible: possible(exposed, qualified),
    };
    match table.and_then(|t| t.get(name)) {
        Some(homes) => one(homes).or_else(|homes| {
            throw(Error::Ambiguous {
                kind,
                range,
                prefix: prefix.map(str::to_owned),
                name: name.to_owned(),
                homes,
            })
        }),
        None => throw(not_found()),
    }
}

impl<'a> Env<'a> {
    /// `addLocals`: `names` bound locally over the environment. A local or
    /// top-level name of the same spelling is shadowed, which is an error;
    /// a foreign one just yields.
    pub fn add_locals(&self, names: &BTreeMap<String, TextRange>) -> R<Env<'a>> {
        let mut env = self.clone();
        let mut errors = Vec::new();
        for name in super::sorted(self.compat, names.keys()) {
            let range = names[name];
            match env.vars.get(name) {
                None | Some(Var::Foreign(_)) => {
                    env.vars.insert(name.clone(), Var::Local(range));
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
        if errors.is_empty() {
            Ok(env)
        } else {
            Err(errors)
        }
    }

    pub fn find_type(
        &self,
        range: TextRange,
        prefix: Option<&str>,
        name: &str,
    ) -> R<&TypeInfo<'a>> {
        find(
            NameKind::Type,
            range,
            &self.types,
            &self.q_types,
            prefix,
            name,
        )
    }

    pub fn find_ctor(
        &self,
        range: TextRange,
        prefix: Option<&str>,
        name: &str,
    ) -> R<&CtorInfo<'a>> {
        find(
            NameKind::Variant,
            range,
            &self.ctors,
            &self.q_ctors,
            prefix,
            name,
        )
    }

    pub fn find_binop(&self, range: TextRange, op: &str) -> R<&BinopInfo<'a>> {
        match self.binops.get(op) {
            Some(homes) => one(homes).or_else(|homes| {
                throw(Error::AmbiguousBinop {
                    range,
                    op: op.to_owned(),
                    homes,
                })
            }),
            None => throw(Error::NotFoundBinop {
                range,
                op: op.to_owned(),
                possible: self.binops.keys().cloned().collect(),
            }),
        }
    }
}
