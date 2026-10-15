//! The canonical AST (`AST/Canonical.hs`): every name resolved to the
//! module that defines it, as interfaces carry types and type inference
//! reads expressions.

use std::collections::BTreeMap;
use std::sync::Arc;

use nomi_base::{Compat, Text, TextRange};

/// A module and the package it is in (`ModuleName.Canonical`). Types
/// carry their home everywhere, so its names are shared, not copied.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Home {
    /// `author/project`.
    pub package: Text,
    pub module: Text,
}

impl Home {
    /// `module` of elm/core.
    pub fn core(module: &str) -> Home {
        Home {
            package: Text::new("elm/core"),
            module: module.into(),
        }
    }

    /// `module` of an application, whose package upstream names
    /// `author/project`.
    pub fn author_project(module: &str) -> Home {
        Home {
            package: Text::new("author/project"),
            module: module.into(),
        }
    }

    /// Whether this is `module` of `package`.
    pub fn is(&self, package: &str, module: &str) -> bool {
        self.package == package && self.module == module
    }

    /// The package's author and project; a package without a `/`, which
    /// no valid name is, is all project.
    #[inline]
    pub fn author_and_project(&self) -> (&str, &str) {
        self.package.split_once('/').unwrap_or(("", &self.package))
    }
}

/// `Can.Type`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    Lambda(Box<Type>, Box<Type>),
    Var(String),
    Type {
        home: Home,
        name: String,
        args: Vec<Type>,
    },
    /// Fields by name, each with its place in the source, and the variable
    /// the record extends.
    Record(BTreeMap<String, FieldType>, Option<String>),
    Unit,
    /// Two or three; Elm has no larger tuples.
    Tuple(Box<Type>, Box<Type>, Option<Box<Type>>),
    /// An alias, applied to its arguments, and what it stands for.
    Alias {
        home: Home,
        name: String,
        args: Vec<(String, Type)>,
        aliased: AliasType,
    },
}

/// What an alias in a type stands for (`Can.AliasType`): its body with
/// the variables still free, or filled in with the arguments. Bodies are
/// shared by every mention of the alias.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AliasType {
    Holey(Arc<Type>),
    Filled(Arc<Type>),
}

impl AliasType {
    /// `dealias`: what the alias, applied to `args`, stands for.
    pub fn dealias(&self, args: &[(String, Type)]) -> Type {
        match self {
            AliasType::Holey(body) => body.substitute(args),
            AliasType::Filled(body) => (**body).clone(),
        }
    }
}

impl Type {
    /// The function from `args`, in order, to `result`.
    pub fn function(args: impl DoubleEndedIterator<Item = Type>, result: Type) -> Type {
        args.rfold(result, |result, arg| {
            Type::Lambda(Box::new(arg), Box::new(result))
        })
    }

    /// The type with each variable `args` names replaced.
    fn substitute(&self, args: &[(String, Type)]) -> Type {
        let go = |t: &Type| t.substitute(args);
        nomi_base::grow(|| match self {
            Type::Var(name) => (args.iter())
                .find(|(var, _)| var == name)
                .map_or_else(|| self.clone(), |(_, arg)| arg.clone()),
            Type::Lambda(a, b) => Type::Lambda(Box::new(go(a)), Box::new(go(b))),
            Type::Type { home, name, args } => Type::Type {
                home: home.clone(),
                name: name.clone(),
                args: args.iter().map(go).collect(),
            },
            Type::Record(fields, ext) => Type::Record(fields_map(fields, go), ext.clone()),
            Type::Unit => Type::Unit,
            Type::Tuple(a, b, c) => Type::Tuple(
                Box::new(go(a)),
                Box::new(go(b)),
                c.as_ref().map(|c| Box::new(go(c))),
            ),
            Type::Alias {
                home,
                name,
                args: alias_args,
                aliased,
            } => Type::Alias {
                home: home.clone(),
                name: name.clone(),
                args: (alias_args.iter())
                    .map(|(var, t)| (var.clone(), go(t)))
                    .collect(),
                aliased: aliased.clone(),
            },
        })
    }

    /// `iteratedDealias`: what the type is under its aliases.
    pub(crate) fn iterated_dealias(mut self) -> Type {
        while let Type::Alias { args, aliased, .. } = self {
            self = aliased.dealias(&args);
        }
        self
    }

    /// `deepDealias`: the type with every alias, at any depth, replaced by
    /// what it stands for.
    pub(crate) fn deep_dealias(&self) -> Type {
        nomi_base::grow(|| match self {
            Type::Alias { args, aliased, .. } => {
                let args: Vec<(String, Type)> = (args.iter())
                    .map(|(var, t)| (var.clone(), t.deep_dealias()))
                    .collect();
                aliased.dealias(&args).deep_dealias()
            }
            Type::Lambda(a, b) => {
                Type::Lambda(Box::new(a.deep_dealias()), Box::new(b.deep_dealias()))
            }
            Type::Type { home, name, args } => Type::Type {
                home: home.clone(),
                name: name.clone(),
                args: args.iter().map(Type::deep_dealias).collect(),
            },
            Type::Record(fields, ext) => {
                Type::Record(fields_map(fields, Type::deep_dealias), ext.clone())
            }
            Type::Tuple(a, b, c) => Type::Tuple(
                Box::new(a.deep_dealias()),
                Box::new(b.deep_dealias()),
                c.as_ref().map(|c| Box::new(c.deep_dealias())),
            ),
            Type::Var(_) | Type::Unit => self.clone(),
        })
    }
}

/// What `main` is, by its type with every alias expanded
/// (`addDefHelp`'s cases).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MainKind {
    /// A `VirtualDom.Node`, as `Html` and `Svg` are.
    Static,
    /// A `Program`, with the types of its flags and messages.
    Program {
        flags: Type,
        message: Type,
    },
    Other,
}

impl Type {
    /// What a `main` of this type is: only the outer type decides, so only
    /// the flags and messages have their aliases expanded.
    pub(crate) fn main_kind(&self) -> MainKind {
        let dealiased;
        let outer = match self {
            Type::Alias { args, aliased, .. } => {
                dealiased = aliased.dealias(args).iterated_dealias();
                &dealiased
            }
            ty => ty,
        };
        match outer {
            Type::Type { home, name, args }
                if args.len() == 1
                    && home.is("elm/virtual-dom", "VirtualDom")
                    && name == "Node" =>
            {
                MainKind::Static
            }
            Type::Type { home, name, args }
                if home.is("elm/core", "Platform") && name == "Program" =>
            {
                match &args[..] {
                    [flags, _, message] => MainKind::Program {
                        flags: flags.deep_dealias(),
                        message: message.deep_dealias(),
                    },
                    _ => MainKind::Other,
                }
            }
            _ => MainKind::Other,
        }
    }
}

/// `Can.fieldsToList`: the fields by their place as written, ties in
/// the order `compat` lists names.
pub fn fields_to_list(
    compat: Compat,
    fields: &BTreeMap<String, FieldType>,
) -> Vec<(&String, &FieldType)> {
    let mut listed: Vec<(&String, &FieldType)> = fields.iter().collect();
    listed.sort_by(|a, b| compat.compare_names(a.0, b.0));
    listed.sort_by_key(|(_, field)| field.order);
    listed
}

/// `fields` with `f` applied to each one's type.
fn fields_map(
    fields: &BTreeMap<String, FieldType>,
    f: impl Fn(&Type) -> Type,
) -> BTreeMap<String, FieldType> {
    (fields.iter())
        .map(|(name, field)| {
            let field = FieldType {
                order: field.order,
                ty: f(&field.ty),
            };
            (name.clone(), field)
        })
        .collect()
}

/// A record field's type and its place among the fields as written; 0
/// for every field of an inferred type (`Can.FieldType`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FieldType {
    pub order: u16,
    pub ty: Type,
}

/// A type with its free variables (`Can.Annotation`). The type is shared:
/// every use of a foreign value carries its annotation, as upstream's
/// expressions point to theirs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Annotation {
    pub free: Vec<String>,
    pub ty: Arc<Type>,
}

/// `Can.Alias`: its parameters and body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias {
    pub vars: Vec<String>,
    pub ty: Arc<Type>,
}

/// `Can.Union`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Union {
    pub vars: Vec<String>,
    pub ctors: Vec<Ctor>,
    pub opts: CtorOpts,
}

/// Which representation codegen may give a union's values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CtorOpts {
    Normal,
    Enum,
    Unbox,
}

/// `Can.Ctor`: a constructor and its arguments; its index is its place
/// among the union's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ctor {
    pub name: String,
    pub args: Vec<Type>,
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
    VarKernel {
        module: String,
        name: String,
    },
    VarForeign {
        home: Home,
        name: String,
        annotation: Annotation,
    },
    /// A constructor, with its union's representation.
    VarCtor {
        opts: CtorOpts,
        home: Home,
        name: String,
        index: usize,
        annotation: Annotation,
    },
    /// A value of `Debug`, by the module using it.
    VarDebug {
        home: Home,
        name: String,
        annotation: Annotation,
    },
    /// An operator as a function.
    VarOperator(Operator),
    Chr(String),
    Str(String),
    Int(i64),
    Float(String),
    List(Vec<Expr>),
    Negate(Box<Expr>),
    /// An operator applied to its operands.
    Binop {
        operator: Operator,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Lambda(Vec<Pattern>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    If(Vec<(Expr, Expr)>, Box<Expr>),
    Let(Box<Def>, Box<Expr>),
    LetRec(Vec<Def>, Box<Expr>),
    /// A destructuring `let`: the pattern, the value it takes apart, and
    /// the body.
    LetDestruct {
        pattern: Pattern,
        value: Box<Expr>,
        body: Box<Expr>,
    },
    Case(Box<Expr>, Vec<(Pattern, Expr)>),
    Accessor(String),
    Access {
        record: Box<Expr>,
        field: String,
        field_region: TextRange,
    },
    /// A record update: the record's name, the record, and each field's
    /// new value.
    Update {
        name: String,
        record: Box<Expr>,
        fields: BTreeMap<String, FieldUpdate>,
    },
    Record(BTreeMap<String, Expr>),
    #[default]
    Unit,
    Tuple(Box<Expr>, Box<Expr>, Option<Box<Expr>>),
    /// A shader's source and the types of its inputs.
    Shader(String, ShaderTypes),
}

/// An operator, the function it stands for, and that function's type
/// (`Can.Binop` and `Can.VarOperator` without their operands).
#[derive(Clone, Debug, PartialEq)]
pub struct Operator {
    pub op: String,
    pub home: Home,
    pub function: String,
    pub annotation: Annotation,
}

/// Deep trees drop on grown stacks, as they canonicalize.
impl Drop for Expr {
    fn drop(&mut self) {
        let kind = std::mem::take(&mut self.kind);
        nomi_base::grow(move || drop(kind));
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
    /// The name it defines.
    pub(crate) fn name(&self) -> &str {
        match self {
            Def::Def { name, .. } | Def::TypedDef { name, .. } => name,
        }
    }

    /// Where its name is.
    pub(crate) fn region(&self) -> TextRange {
        match self {
            Def::Def { region, .. } | Def::TypedDef { region, .. } => *region,
        }
    }

    /// Whether it is a function, which a cycle may pass through.
    pub(crate) fn has_params(&self) -> bool {
        match self {
            Def::Def { params, .. } => !params.is_empty(),
            Def::TypedDef { params, .. } => !params.is_empty(),
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
    Bool(Arc<Union>, bool),
    Chr(String),
    Str(String),
    Int(i64),
    /// A constructor, with its union, which every pattern of its
    /// constructors shares.
    Ctor {
        home: Home,
        type_name: String,
        union: Arc<Union>,
        name: String,
        index: usize,
        args: Vec<PatternCtorArg>,
    },
}

impl Drop for Pattern {
    fn drop(&mut self) {
        let kind = std::mem::take(&mut self.kind);
        nomi_base::grow(move || drop(kind));
    }
}

/// `Can.PatternCtorArg`: an argument of a constructor pattern and the type
/// the constructor gives it; its index is its place among the arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct PatternCtorArg {
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
    /// An effect manager: where its three functions are defined, and what
    /// it manages.
    Manager {
        init: TextRange,
        on_effects: TextRange,
        on_self_msg: TextRange,
        manager: Manager,
    },
}

/// `Can.Port`: its direction, its free variables, its payload and its
/// whole type.
#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub direction: PortDirection,
    pub free: Vec<String>,
    pub payload: Type,
    pub func: Arc<Type>,
}

/// Which way a port carries values: from JavaScript (a subscription) or
/// to it (a command).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortDirection {
    Incoming,
    Outgoing,
}

/// `Can.Manager`: the effect types a manager handles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Manager {
    Cmd(String),
    Sub(String),
    Fx { cmd: String, sub: String },
}
