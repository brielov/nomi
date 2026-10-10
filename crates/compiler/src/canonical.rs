//! The canonical AST (`AST/Canonical.hs`): every name resolved to the
//! module that defines it, as interfaces carry types and type inference
//! reads expressions.

use std::collections::BTreeMap;

use nomi_base::TextRange;

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

/// An expression with its region (`Can.Expr`). The region is its node's,
/// leading trivia included, as canonicalization's errors keep it.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub region: TextRange,
    pub kind: ExprKind,
}

/// `Can.Expr_`. A literal keeps its text as written.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ExprKind {
    VarLocal(String),
    VarTopLevel(Home, String),
    /// A kernel module's value, the module without `Elm.Kernel.`.
    VarKernel(String, String),
    VarForeign(Home, String, Annotation),
    /// A constructor: its union's representation, home, name, index and
    /// type.
    VarCtor(CtorOpts, Home, String, usize, Annotation),
    /// A value of `Debug`, by the module using it.
    VarDebug(Home, String, Annotation),
    /// An operator as a function: the operator, and the function it
    /// stands for.
    VarOperator(String, Home, String, Annotation),
    Chr(String),
    Str(String),
    Int(i64),
    Float(String),
    List(Vec<Expr>),
    Negate(Box<Expr>),
    /// An operator applied: the operator, the function it stands for,
    /// and its operands.
    Binop(String, Home, String, Annotation, Box<Expr>, Box<Expr>),
    Lambda(Vec<Pattern>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    If(Vec<(Expr, Expr)>, Box<Expr>),
    Let(Box<Def>, Box<Expr>),
    LetRec(Vec<Def>, Box<Expr>),
    LetDestruct(Pattern, Box<Expr>, Box<Expr>),
    Case(Box<Expr>, Vec<(Pattern, Expr)>),
    Accessor(String),
    /// The record and the field, with its region.
    Access(Box<Expr>, TextRange, String),
    /// The record's name, the record, and each field's new value.
    Update(String, Box<Expr>, BTreeMap<String, FieldUpdate>),
    Record(BTreeMap<String, Expr>),
    #[default]
    Unit,
    Tuple(Box<Expr>, Box<Expr>, Option<Box<Expr>>),
    /// A shader's source and the types of its inputs.
    Shader(String, ShaderTypes),
}

/// Deep trees drop on grown stacks, as they canonicalize.
impl Drop for Expr {
    fn drop(&mut self) {
        let kind = std::mem::take(&mut self.kind);
        stacker::maybe_grow(64 * 1024, 1024 * 1024, move || drop(kind));
    }
}

/// `Can.FieldUpdate`.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldUpdate {
    pub region: TextRange,
    pub value: Expr,
}

/// `Can.Def`: a definition, with its parameters typed where it is
/// annotated.
#[derive(Clone, Debug, PartialEq)]
pub enum Def {
    Def {
        name: String,
        region: TextRange,
        params: Vec<Pattern>,
        body: Expr,
    },
    TypedDef {
        name: String,
        region: TextRange,
        /// The annotation's free variables.
        free: Vec<String>,
        params: Vec<(Pattern, Type)>,
        body: Expr,
        /// What the body must be, once the parameters are taken.
        result: Type,
    },
}

impl Def {
    pub fn name(&self) -> &str {
        match self {
            Def::Def { name, .. } | Def::TypedDef { name, .. } => name,
        }
    }

    pub fn region(&self) -> TextRange {
        match self {
            Def::Def { region, .. } | Def::TypedDef { region, .. } => *region,
        }
    }
}

/// One step of `Can.Decls`: a value alone, or values that use each
/// other.
#[derive(Clone, Debug, PartialEq)]
pub enum Decl {
    Declare(Def),
    DeclareRec(Def, Vec<Def>),
}

/// A pattern with its region (`Can.Pattern`).
#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    pub region: TextRange,
    pub kind: PatternKind,
}

/// `Can.Pattern_`.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum PatternKind {
    #[default]
    Anything,
    Var(String),
    Record(Vec<String>),
    Alias(Box<Pattern>, String),
    Unit,
    Tuple(Box<Pattern>, Box<Pattern>, Option<Box<Pattern>>),
    List(Vec<Pattern>),
    Cons(Box<Pattern>, Box<Pattern>),
    /// `True` or `False`, with `Bool`'s union.
    Bool(Union, bool),
    Chr(String),
    Str(String),
    Int(i64),
    Ctor {
        home: Home,
        type_name: String,
        union: Union,
        name: String,
        index: usize,
        args: Vec<PatternCtorArg>,
    },
}

impl Drop for Pattern {
    fn drop(&mut self) {
        let kind = std::mem::take(&mut self.kind);
        stacker::maybe_grow(64 * 1024, 1024 * 1024, move || drop(kind));
    }
}

/// `Can.PatternCtorArg`: an argument of a constructor pattern, its index
/// and the type the constructor gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct PatternCtorArg {
    pub index: usize,
    pub ty: Type,
    pub arg: Pattern,
}

/// `Shader.Types`: a shader's attributes, uniforms and varyings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShaderTypes {
    pub attribute: BTreeMap<String, ShaderType>,
    pub uniform: BTreeMap<String, ShaderType>,
    pub varying: BTreeMap<String, ShaderType>,
}

/// `Shader.Type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderType {
    Int,
    Float,
    V2,
    V3,
    V4,
    M4,
    Texture,
}

/// `Can.Effects`: what a module's header makes of it.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Effects {
    #[default]
    NoEffects,
    Ports(BTreeMap<String, Port>),
    /// The regions of `init`, `onEffects` and `onSelfMsg`, and what the
    /// manager manages.
    Manager(TextRange, TextRange, TextRange, Manager),
}

/// `Can.Port`: its free variables, its payload and its whole type.
#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub incoming: bool,
    pub free: Vec<String>,
    pub payload: Type,
    pub func: Type,
}

/// `Can.Manager`: the effect types a manager handles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Manager {
    Cmd(String),
    Sub(String),
    Fx(String, String),
}
