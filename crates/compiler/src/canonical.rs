//! Canonical types (`AST/Canonical.hs`): every name resolved to the
//! module that defines it, as interfaces carry them and type inference
//! (M6) reads them.

use std::collections::BTreeMap;

/// A module and the package it is in (`ModuleName.Canonical`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Home {
    /// `author/project`.
    pub package: String,
    pub module: String,
}

/// `Can.Type`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Lambda(Box<Type>, Box<Type>),
    Var(String),
    Type(Home, String, Vec<Type>),
    /// Fields by name, each with its place in the source, and the variable
    /// the record extends.
    Record(BTreeMap<String, FieldType>, Option<String>),
    Unit,
    /// Two or three; Elm has no larger tuples.
    Tuple(Box<Type>, Box<Type>, Option<Box<Type>>),
    /// An alias, applied to its arguments, and what it stands for.
    Alias(Home, String, Vec<(String, Type)>, AliasType),
}

/// What an alias in a type stands for (`Can.AliasType`): its body with
/// the variables still free, or filled in with the arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AliasType {
    Holey(Box<Type>),
    Filled(Box<Type>),
}

/// A record field's type and its place among the fields as written; 0
/// for every field of an inferred type (`Can.FieldType`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldType {
    pub order: u16,
    pub ty: Type,
}

/// A type with its free variables (`Can.Annotation`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Annotation {
    pub free: Vec<String>,
    pub ty: Type,
}

/// `Can.Alias`: its parameters and body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias {
    pub vars: Vec<String>,
    pub ty: Type,
}

/// `Can.Union`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Union {
    pub vars: Vec<String>,
    pub ctors: Vec<Ctor>,
    /// How many constructors the type has, kept when a closed export
    /// hides them, for exhaustiveness checking.
    pub count: usize,
    pub opts: CtorOpts,
}

/// Which representation codegen may give a union's values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CtorOpts {
    Normal,
    Enum,
    Unbox,
}

/// `Can.Ctor`: a constructor, its index among the union's, and its
/// arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ctor {
    pub name: String,
    pub index: usize,
    pub args: Vec<Type>,
}

/// An operator's associativity (`Binop.Associativity`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Associativity {
    Left,
    Non,
    Right,
}
