//! The optimized AST (`AST/Optimized.hs`): what code generation reads,
//! each top-level definition a node of a graph with the globals it needs.
//! elm writes a module's graph to `elm-stuff/0.19.x/<Module>.elmo` and
//! its dependencies' to `o.dat`; [`binary`] writes them the same way.
//!
//! Maps and sets are keyed by plain strings here; each release's order
//! of names is applied where they are written.

pub mod binary;

use std::collections::{BTreeMap, BTreeSet};

use crate::canonical::{CtorOpts, Home, Type};

/// `Opt.Expr`.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Bool(bool),
    /// A character: its literal's text, escapes kept, in 0.19.1, and the
    /// character itself in later releases.
    Chr(String),
    /// A string as its literal's text, escapes kept.
    Str(String),
    Int(i64),
    /// A float as its literal's text.
    Float(String),
    VarLocal(String),
    VarGlobal(Global),
    /// A constructor of an enumeration, by its index.
    VarEnum(Global, usize),
    VarBox(Global),
    VarCycle(Home, String),
    /// A `Debug` value: its name, the module using it, where, and the name
    /// of the pattern variable a `todo` in a case is about.
    VarDebug(String, Home, Region, Option<String>),
    /// A kernel module's value: the module without `Elm.Kernel.`, and the
    /// name.
    VarKernel(String, String),
    List(Vec<Expr>),
    Function(Vec<String>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    TailCall(String, Vec<(String, Expr)>),
    If(Vec<(Expr, Expr)>, Box<Expr>),
    Let(Box<Def>, Box<Expr>),
    Destruct(Destructor, Box<Expr>),
    /// The variable the cases test, the root it was taken from, the
    /// decisions, and the jumps' bodies by target.
    Case(String, String, Box<Decider<Choice>>, Vec<(usize, Expr)>),
    Accessor(String),
    Access(Box<Expr>, String),
    Update(Box<Expr>, BTreeMap<String, Expr>),
    Record(BTreeMap<String, Expr>),
    Unit,
    /// Two or three; 0.19.3 calls them a pair and a triple.
    Tuple(Box<Expr>, Box<Expr>, Option<Box<Expr>>),
    /// A shader's source, and the names of its attributes and its
    /// uniforms.
    Shader(Vec<u8>, BTreeSet<String>, BTreeSet<String>),
}

/// `Opt.Global`: a top-level name and its module.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Global {
    pub home: Home,
    pub name: String,
}

/// `A.Region`, as the release keeps it: rows and columns from 1 in 0.19.1,
/// from 0 later (`code::Source::region`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub start: (u32, u32),
    pub end: (u32, u32),
}

/// `Opt.Def`.
#[derive(Clone, Debug, PartialEq)]
pub enum Def {
    Def(String, Expr),
    TailDef(String, Vec<String>, Expr),
}

/// `Opt.Destructor`: a name bound to a path into a value.
#[derive(Clone, Debug, PartialEq)]
pub struct Destructor {
    pub name: String,
    pub path: Path,
}

/// `Opt.Path`.
#[derive(Clone, Debug, PartialEq)]
pub enum Path {
    Index(usize, Box<Path>),
    Field(String, Box<Path>),
    Unbox(Box<Path>),
    Root(String),
}

/// `Opt.Decider`.
#[derive(Clone, Debug, PartialEq)]
pub enum Decider<T> {
    Leaf(T),
    Chain {
        tests: Vec<(DtPath, Test)>,
        success: Box<Decider<T>>,
        failure: Box<Decider<T>>,
    },
    FanOut {
        path: DtPath,
        tests: Vec<(Test, Decider<T>)>,
        fallback: Box<Decider<T>>,
    },
}

/// `Opt.Choice`.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    Inline(Expr),
    Jump(usize),
}

/// `DecisionTree.Path`: where in the tested value a test looks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DtPath {
    Index(usize, Box<DtPath>),
    Unbox(Box<DtPath>),
    Empty,
}

/// `DecisionTree.Test`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Test {
    /// A constructor: its home, name, index and how many its union has.
    IsCtor(Home, String, usize, i64, CtorOpts),
    IsCons,
    IsNil,
    IsTuple,
    /// As [`Expr::Chr`] has it.
    IsChr(String),
    IsStr(String),
    IsInt(i64),
    IsBool(bool),
}

/// `Opt.GlobalGraph`: every definition of a set of modules, and how
/// often each record field is used, which `--optimize` shortens by.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlobalGraph {
    pub nodes: BTreeMap<Global, Node>,
    pub fields: BTreeMap<String, i64>,
}

/// `Opt.LocalGraph`: one module's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocalGraph {
    pub main: Option<Main>,
    pub nodes: BTreeMap<Global, Node>,
    pub fields: BTreeMap<String, i64>,
}

/// `Opt.Main`: a program without flags, or the type of its flags and
/// their decoder.
#[derive(Clone, Debug, PartialEq)]
pub enum Main {
    Static,
    Dynamic { message: Type, decoder: Box<Expr> },
}

/// `Opt.Node`: a definition and the globals it needs.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Define(Expr, BTreeSet<Global>),
    DefineTailFunc(Vec<String>, Expr, BTreeSet<Global>),
    /// A constructor: its index and arity.
    Ctor(usize, i64),
    Enum(usize),
    Box,
    Link(Global),
    /// Mutually recursive definitions: the names, the values among them,
    /// the functions, and what they need.
    Cycle(Vec<String>, Vec<(String, Expr)>, Vec<Def>, BTreeSet<Global>),
    Manager(EffectsType),
    Kernel(Vec<Chunk>, BTreeSet<Global>),
    PortIncoming(Expr, BTreeSet<Global>),
    PortOutgoing(Expr, BTreeSet<Global>),
}

/// `Opt.EffectsType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectsType {
    Cmd,
    Sub,
    Fx,
}

/// `Kernel.Chunk`: a piece of a kernel module's JavaScript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chunk {
    Js(Vec<u8>),
    ElmVar(Home, String),
    /// Another kernel module's value: the module, short, and the name.
    JsVar(String, String),
    ElmField(String),
    JsField(i64),
    JsEnum(i64),
    Debug,
    Prod,
}
