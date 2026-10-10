//! What a type is expected to be, and why (`Reporting/Error/Type.hs`'s
//! expectations and categories): constraints carry these, and a failed
//! one becomes a type error that says where the expectation came from.

use std::collections::BTreeMap;

use nomi_base::TextRange;

/// `Expected`: a type an expression must have, from nowhere in
/// particular, from its context, or from an annotation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expected<T> {
    NoExpectation(T),
    FromContext(TextRange, Context, T),
    /// The annotated name, its arity, and where in its body.
    FromAnnotation(String, usize, SubContext, T),
}

impl<T> Expected<T> {
    pub fn ty(&self) -> &T {
        match self {
            Expected::NoExpectation(t)
            | Expected::FromContext(_, _, t)
            | Expected::FromAnnotation(_, _, _, t) => t,
        }
    }

    /// `typeReplace`: the same expectation of another type.
    pub fn replace<U>(&self, ty: U) -> Expected<U> {
        match self {
            Expected::NoExpectation(_) => Expected::NoExpectation(ty),
            Expected::FromContext(region, context, _) => {
                Expected::FromContext(*region, context.clone(), ty)
            }
            Expected::FromAnnotation(name, arity, sub, _) => {
                Expected::FromAnnotation(name.clone(), *arity, *sub, ty)
            }
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Expected<U> {
        match self {
            Expected::NoExpectation(t) => Expected::NoExpectation(f(t)),
            Expected::FromContext(region, context, t) => {
                Expected::FromContext(region, context, f(t))
            }
            Expected::FromAnnotation(name, arity, sub, t) => {
                Expected::FromAnnotation(name, arity, sub, f(t))
            }
        }
    }
}

/// `Context`. Indexes count from zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Context {
    ListEntry(usize),
    Negate,
    OpLeft(String),
    OpRight(String),
    IfCondition,
    IfBranch(usize),
    CaseBranch(usize),
    CallArity(MaybeName, usize),
    CallArg(MaybeName, usize),
    /// The record's region, its name if it is a variable, the field's
    /// region and the field.
    RecordAccess(TextRange, Option<String>, TextRange, String),
    /// The record, and each updated field's region.
    RecordUpdateKeys(String, BTreeMap<String, TextRange>),
    RecordUpdateValue(String),
    Destructure,
}

/// `SubContext`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubContext {
    TypedIfBranch(usize),
    TypedCaseBranch(usize),
    TypedBody,
}

/// `MaybeName`: what is called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaybeName {
    FuncName(String),
    CtorName(String),
    OpName(String),
    NoName,
}

/// `Category`: what kind of expression has the type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Category {
    List,
    Number,
    Float,
    String,
    Char,
    If,
    Case,
    CallResult(MaybeName),
    Lambda,
    Accessor(String),
    Access(String),
    Record,
    Tuple,
    Unit,
    Shader,
    Effects,
    Local(String),
    Foreign(String),
}

/// `PExpected`: a type a pattern must match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PExpected<T> {
    PNoExpectation(T),
    PFromContext(TextRange, PContext, T),
}

impl<T> PExpected<T> {
    pub fn ty(&self) -> &T {
        match self {
            PExpected::PNoExpectation(t) | PExpected::PFromContext(_, _, t) => t,
        }
    }

    /// `ptypeReplace`.
    pub fn replace<U>(&self, ty: U) -> PExpected<U> {
        match self {
            PExpected::PNoExpectation(_) => PExpected::PNoExpectation(ty),
            PExpected::PFromContext(region, context, _) => {
                PExpected::PFromContext(*region, context.clone(), ty)
            }
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> PExpected<U> {
        match self {
            PExpected::PNoExpectation(t) => PExpected::PNoExpectation(f(t)),
            PExpected::PFromContext(region, context, t) => {
                PExpected::PFromContext(region, context, f(t))
            }
        }
    }
}

/// `PContext`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PContext {
    PTypedArg(String, usize),
    PCaseMatch(usize),
    PCtorArg(String, usize),
    PListEntry(usize),
    PTail,
}

/// `PCategory`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PCategory {
    PRecord,
    PUnit,
    PTuple,
    PList,
    PCtor(String),
    PInt,
    PStr,
    PChr,
    PBool,
}
