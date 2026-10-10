//! Types as type errors show them (`Type/Error.hs`): every variable
//! named, a function's arguments flat, and a type that contains itself
//! cut short.

use std::collections::BTreeMap;

use crate::canonical::Home;

/// `T.Type`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    /// The first argument, the second, then the rest and the result.
    Lambda(Box<Type>, Box<Type>, Vec<Type>),
    Infinite,
    Error,
    FlexVar(String),
    FlexSuper(Super, String),
    RigidVar(String),
    RigidSuper(Super, String),
    Type(Home, String, Vec<Type>),
    Record(BTreeMap<String, Type>, Extension),
    Unit,
    Tuple(Box<Type>, Box<Type>, Option<Box<Type>>),
    Alias(Home, String, Vec<(String, Type)>, Box<Type>),
}

/// `T.Super`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Super {
    Number,
    Comparable,
    Appendable,
    CompAppend,
}

/// `T.Extension`: what a record type extends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Extension {
    Closed,
    FlexOpen(String),
    RigidOpen(String),
}

/// `iteratedDealias`: what `ty` stands for under all its aliases.
pub fn iterated_dealias(ty: Type) -> Type {
    match ty {
        Type::Alias(_, _, _, real) => iterated_dealias(*real),
        ty => ty,
    }
}
