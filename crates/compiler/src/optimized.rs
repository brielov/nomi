//! The optimized AST (`AST/Optimized.hs`): what code generation reads,
//! each top-level definition a node of a graph with the globals it needs.
//! elm writes a module's graph to `elm-stuff/0.19.x/<Module>.elmo` and
//! its dependencies' to `o.dat`; [`binary`] writes them the same way.
//!
//! Maps and sets are keyed by plain strings here; each release's order
//! of names is applied where they are written.

pub mod binary;

use std::collections::{BTreeMap, BTreeSet};

use nomi_base::diagnostic::code::Region;

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
    /// A `Debug` value, by the module using it, where; `unhandled` is the
    /// pattern variable a `todo` in a case's branch is about.
    VarDebug {
        name: String,
        home: Home,
        region: Region,
        unhandled: Option<String>,
    },
    /// A kernel module's value, the module without `Elm.Kernel.`.
    VarKernel {
        module: String,
        name: String,
    },
    List(Vec<Expr>),
    Function(Vec<String>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    TailCall(String, Vec<(String, Expr)>),
    If(Vec<(Expr, Expr)>, Box<Expr>),
    Let(Box<Def>, Box<Expr>),
    Destruct(Destructor, Box<Expr>),
    /// A `case`: `label`, the variable the cases test, the root it was
    /// taken from, the decisions, and the jumps' bodies by target.
    Case {
        label: String,
        root: String,
        decider: Box<Decider<Choice>>,
        jumps: Vec<(usize, Expr)>,
    },
    Accessor(String),
    Access(Box<Expr>, String),
    Update(Box<Expr>, BTreeMap<String, Expr>),
    Record(BTreeMap<String, Expr>),
    Unit,
    /// Two or three; 0.19.3 calls them a pair and a triple.
    Tuple(Box<Expr>, Box<Expr>, Option<Box<Expr>>),
    /// A shader's source, and the names of its inputs.
    Shader {
        source: Vec<u8>,
        attributes: BTreeSet<String>,
        uniforms: BTreeSet<String>,
    },
}

/// `Opt.Global`: a top-level name and its module.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Global {
    pub home: Home,
    pub name: String,
}

impl Global {
    /// `toKernelGlobal`: the kernel module `short` (without `Elm.Kernel.`)
    /// as a global, named `$`, or `#0` in 0.19.3.
    pub fn kernel(compat: nomi_base::Compat, short: &str) -> Global {
        let name = if compat == nomi_base::Compat::V0_19_3 {
            "#0"
        } else {
            "$"
        };
        Global {
            home: Home {
                package: "elm/kernel".into(),
                module: short.into(),
            },
            name: name.to_owned(),
        }
    }
}

/// `Opt.Def`.
#[derive(Clone, Debug, PartialEq)]
pub enum Def {
    Def(String, Expr),
    TailDef {
        name: String,
        params: Vec<String>,
        body: Expr,
    },
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

/// `DecisionTree.Path`: where in the tested value a test looks. Paths
/// share their prefixes, as the case compiler copies them at every node,
/// and a shared prefix is equal without a look.
#[expect(
    clippy::derived_hash_with_manual_eq,
    reason = "the equality below is the structural one, so the derived hash agrees"
)]
#[derive(Clone, Debug, Eq, Hash)]
pub enum DtPath {
    Index(usize, std::sync::Arc<DtPath>),
    Unbox(std::sync::Arc<DtPath>),
    Empty,
}

impl PartialEq for DtPath {
    fn eq(&self, other: &DtPath) -> bool {
        let same = |a: &std::sync::Arc<DtPath>, b: &std::sync::Arc<DtPath>| {
            std::sync::Arc::ptr_eq(a, b) || a == b
        };
        match (self, other) {
            (DtPath::Index(i, a), DtPath::Index(j, b)) => i == j && same(a, b),
            (DtPath::Unbox(a), DtPath::Unbox(b)) => same(a, b),
            (DtPath::Empty, DtPath::Empty) => true,
            _ => false,
        }
    }
}

/// `DecisionTree.Test`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Test {
    /// A constructor, with how many its union has.
    IsCtor {
        home: Home,
        name: String,
        index: usize,
        count: i64,
        opts: CtorOpts,
    },
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

impl LocalGraph {
    /// `fields`' uses added to the graph's.
    pub(crate) fn count_fields(&mut self, fields: impl IntoIterator<Item = (String, i64)>) {
        for (field, n) in fields {
            *self.fields.entry(field).or_default() += n;
        }
    }
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
    DefineTailFunc {
        params: Vec<String>,
        body: Expr,
        deps: BTreeSet<Global>,
    },
    /// A constructor: its index and arity.
    Ctor(usize, i64),
    Enum(usize),
    Box,
    Link(Global),
    /// Mutually recursive definitions: their names, the values among
    /// them, the functions, and what they need.
    Cycle {
        names: Vec<String>,
        values: Vec<(String, Expr)>,
        functions: Vec<Def>,
        deps: BTreeSet<Global>,
    },
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
    /// Another kernel module's value, the module without `Elm.Kernel.`.
    JsVar {
        module: String,
        name: String,
    },
    ElmField(String),
    JsField(i64),
    JsEnum(i64),
    Debug,
    Prod,
}
