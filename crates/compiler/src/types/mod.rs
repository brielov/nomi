//! Type inference (`Type/*.hs`), ported from 0.19.1's, which 0.19.2
//! keeps; 0.19.3's changes come in where they show. This module holds
//! the type variables, kept in a union-find store with what each stands
//! for, its rank and its mark, and the constraints inference solves
//! (`Type/Type.hs`).

pub mod constrain;
pub mod error;
pub mod expected;
pub mod instantiate;
pub mod localizer;
mod names;
pub mod occurs;
pub mod render;
pub mod solve;
pub mod unify;
pub mod union_find;

use std::collections::BTreeMap;

use nomi_base::TextRange;

pub use names::{from_type_variable, from_type_variable_scheme};

use crate::canonical::{self, Home};
use expected::{Category, Expected, PCategory, PExpected};
use union_find::{Point, UnionFind};

/// A type variable: a point in the [`Variables`] store.
pub type Variable = Point;

/// `Constraint`.
#[derive(Clone, Debug)]
pub enum Constraint {
    True,
    SaveTheEnvironment,
    Equal(TextRange, Category, Type, Expected<Type>),
    Local(TextRange, String, Expected<Type>),
    Foreign(
        TextRange,
        String,
        Box<canonical::Annotation>,
        Expected<Type>,
    ),
    Pattern(TextRange, PCategory, Type, PExpected<Type>),
    And(Vec<Constraint>),
    Let {
        rigid_vars: Vec<Variable>,
        flex_vars: Vec<Variable>,
        /// The names the header binds, each with its region and type.
        header: BTreeMap<String, (TextRange, Type)>,
        header_con: Box<Constraint>,
        body_con: Box<Constraint>,
    },
}

/// `exists`: `constraint` over the fresh `flex_vars`.
pub fn exists(flex_vars: Vec<Variable>, constraint: Constraint) -> Constraint {
    Constraint::Let {
        rigid_vars: Vec::new(),
        flex_vars,
        header: BTreeMap::new(),
        header_con: Box::new(constraint),
        body_con: Box::new(Constraint::True),
    }
}

/// `FlatType`: one layer of a type, its parts variables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlatType {
    App1(Home, String, Vec<Variable>),
    Fun1(Variable, Variable),
    EmptyRecord1,
    Record1(BTreeMap<String, Variable>, Variable),
    Unit1,
    Tuple1(Variable, Variable, Option<Variable>),
}

/// `Type`: a type as constraints write it, parts of it variables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    PlaceHolder(String),
    Alias(Home, String, Vec<(String, Type)>, Box<Type>),
    Var(Variable),
    App(Home, String, Vec<Type>),
    Fun(Box<Type>, Box<Type>),
    EmptyRecord,
    Record(BTreeMap<String, Type>, Box<Type>),
    Unit,
    Tuple(Box<Type>, Box<Type>, Option<Box<Type>>),
}

/// `==>`: a function type.
pub fn fun(arg: Type, result: Type) -> Type {
    Type::Fun(Box::new(arg), Box::new(result))
}

/// `Descriptor`: what a variable stands for, the rank of the `let` that
/// binds it, a mark for traversals, and its copy while instantiating.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub content: Content,
    pub rank: u32,
    pub mark: Mark,
    pub copy: Option<Variable>,
}

impl Descriptor {
    /// `makeDescriptor`.
    pub fn new(content: Content) -> Descriptor {
        Descriptor {
            content,
            rank: NO_RANK,
            mark: NO_MARK,
            copy: None,
        }
    }
}

/// `Content`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    FlexVar(Option<String>),
    FlexSuper(SuperType, Option<String>),
    RigidVar(String),
    RigidSuper(SuperType, String),
    Structure(FlatType),
    Alias(Home, String, Vec<(String, Variable)>, Variable),
    Error,
}

/// `SuperType`: the constrained type variables Elm has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuperType {
    Number,
    Comparable,
    Appendable,
    CompAppend,
}

impl SuperType {
    /// `toSuper`: what a type variable's name says it ranges over.
    pub fn of_name(name: &str) -> Option<SuperType> {
        if name.starts_with("number") {
            Some(SuperType::Number)
        } else if name.starts_with("comparable") {
            Some(SuperType::Comparable)
        } else if name.starts_with("appendable") {
            Some(SuperType::Appendable)
        } else if name.starts_with("compappend") {
            Some(SuperType::CompAppend)
        } else {
            None
        }
    }

    /// `superToSuper`.
    pub fn to_error(self) -> error::Super {
        match self {
            SuperType::Number => error::Super::Number,
            SuperType::Comparable => error::Super::Comparable,
            SuperType::Appendable => error::Super::Appendable,
            SuperType::CompAppend => error::Super::CompAppend,
        }
    }
}

/// `noRank`: a variable no `let` has bound yet.
pub const NO_RANK: u32 = 0;
/// `outermostRank`.
pub const OUTERMOST_RANK: u32 = 1;

/// `Mark`: what a traversal has visited.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mark(u32);

/// `getVarNamesMark`.
pub const GET_VAR_NAMES_MARK: Mark = Mark(0);
/// `occursMark`.
pub const OCCURS_MARK: Mark = Mark(1);
/// `noMark`, which every fresh variable has.
pub const NO_MARK: Mark = Mark(2);

impl Mark {
    /// `nextMark`.
    pub fn next(self) -> Mark {
        Mark(self.0 + 1)
    }
}

fn basics(name: &str) -> Type {
    Type::App(core("Basics"), name.to_owned(), Vec::new())
}

/// `elm/core`'s module `module`.
pub fn core(module: &str) -> Home {
    Home {
        package: "elm/core".to_owned(),
        module: module.to_owned(),
    }
}

pub fn int() -> Type {
    basics("Int")
}

pub fn float() -> Type {
    basics("Float")
}

pub fn bool() -> Type {
    basics("Bool")
}

pub fn never() -> Type {
    basics("Never")
}

pub fn char() -> Type {
    Type::App(core("Char"), "Char".to_owned(), Vec::new())
}

pub fn string() -> Type {
    Type::App(core("String"), "String".to_owned(), Vec::new())
}

fn package_type(package: &str, module: &str, name: &str) -> Type {
    let home = Home {
        package: package.to_owned(),
        module: module.to_owned(),
    };
    Type::App(home, name.to_owned(), Vec::new())
}

/// WebGL's types, which shaders' attributes, uniforms and varyings have.
pub fn vec2() -> Type {
    package_type("elm-explorations/linear-algebra", "Math.Vector2", "Vec2")
}

pub fn vec3() -> Type {
    package_type("elm-explorations/linear-algebra", "Math.Vector3", "Vec3")
}

pub fn vec4() -> Type {
    package_type("elm-explorations/linear-algebra", "Math.Vector4", "Vec4")
}

pub fn mat4() -> Type {
    package_type("elm-explorations/linear-algebra", "Math.Matrix4", "Mat4")
}

pub fn texture() -> Type {
    package_type("elm-explorations/webgl", "WebGL.Texture", "Texture")
}

/// The type variables of one inference, each with its descriptor.
#[derive(Clone, Debug, Default)]
pub struct Variables {
    store: UnionFind<Descriptor>,
    /// Where constraint generation records each expression's type, when
    /// asked to (plan 6.8): its region and the type it is expected to
    /// have, which solving makes its type.
    pub recorded: Option<Vec<(TextRange, Type)>>,
}

impl Variables {
    pub fn fresh(&mut self, content: Content) -> Variable {
        self.store.fresh(Descriptor::new(content))
    }

    /// `UF.fresh` of a whole descriptor.
    pub fn fresh_descriptor(&mut self, descriptor: Descriptor) -> Variable {
        self.store.fresh(descriptor)
    }

    /// `mkFlexVar`.
    pub fn flex_var(&mut self) -> Variable {
        self.fresh(Content::FlexVar(None))
    }

    /// `mkFlexNumber`.
    pub fn flex_number(&mut self) -> Variable {
        self.fresh(Content::FlexSuper(SuperType::Number, None))
    }

    /// `nameToFlex`.
    pub fn name_to_flex(&mut self, name: &str) -> Variable {
        let content = match SuperType::of_name(name) {
            Some(super_type) => Content::FlexSuper(super_type, Some(name.to_owned())),
            None => Content::FlexVar(Some(name.to_owned())),
        };
        self.fresh(content)
    }

    /// `nameToRigid`.
    pub fn name_to_rigid(&mut self, name: &str) -> Variable {
        let content = match SuperType::of_name(name) {
            Some(super_type) => Content::RigidSuper(super_type, name.to_owned()),
            None => Content::RigidVar(name.to_owned()),
        };
        self.fresh(content)
    }

    pub fn get(&mut self, var: Variable) -> &Descriptor {
        self.store.get(var)
    }

    pub fn get_mut(&mut self, var: Variable) -> &mut Descriptor {
        self.store.get_mut(var)
    }

    pub fn content(&mut self, var: Variable) -> Content {
        self.get(var).content.clone()
    }

    pub fn set(&mut self, var: Variable, descriptor: Descriptor) {
        self.store.set(var, descriptor);
    }

    pub fn union(&mut self, a: Variable, b: Variable, descriptor: Descriptor) {
        self.store.union(a, b, descriptor);
    }

    pub fn equivalent(&mut self, a: Variable, b: Variable) -> bool {
        self.store.equivalent(a, b)
    }

    pub fn redundant(&self, var: Variable) -> bool {
        self.store.redundant(var)
    }

    /// `toAnnotation`: the type `var` stands for, each unnamed variable
    /// named, with its free variables.
    pub fn to_annotation(&mut self, var: Variable) -> canonical::Annotation {
        names::to_annotation(self, var)
    }

    /// `toErrorType`: the type `var` stands for, as an error shows it.
    pub fn to_error_type(&mut self, var: Variable) -> error::Type {
        names::to_error_type(self, var)
    }

    /// A variable standing for `ty`, outside the solver's pools: for
    /// reading a type once solving is done.
    pub fn type_to_var(&mut self, ty: &Type) -> Variable {
        let structure = |vars: &mut Variables, flat| vars.fresh(Content::Structure(flat));
        match ty {
            Type::Var(v) => *v,
            Type::App(home, name, args) => {
                let args = args.iter().map(|a| self.type_to_var(a)).collect();
                structure(self, FlatType::App1(home.clone(), name.clone(), args))
            }
            Type::Fun(a, b) => {
                let a = self.type_to_var(a);
                let b = self.type_to_var(b);
                structure(self, FlatType::Fun1(a, b))
            }
            Type::Alias(home, name, args, real) => {
                let args = args
                    .iter()
                    .map(|(n, a)| (n.clone(), self.type_to_var(a)))
                    .collect();
                let real = self.type_to_var(real);
                self.fresh(Content::Alias(home.clone(), name.clone(), args, real))
            }
            // Constraint generation makes no place holders.
            Type::PlaceHolder(_) => self.fresh(Content::Error),
            Type::Record(fields, ext) => {
                let fields = fields
                    .iter()
                    .map(|(n, f)| (n.clone(), self.type_to_var(f)))
                    .collect();
                let ext = self.type_to_var(ext);
                structure(self, FlatType::Record1(fields, ext))
            }
            Type::EmptyRecord => structure(self, FlatType::EmptyRecord1),
            Type::Unit => structure(self, FlatType::Unit1),
            Type::Tuple(a, b, c) => {
                let a = self.type_to_var(a);
                let b = self.type_to_var(b);
                let c = c.as_ref().map(|c| self.type_to_var(c));
                structure(self, FlatType::Tuple1(a, b, c))
            }
        }
    }
}

/// A module's top-level annotations, or its type errors, the latest
/// first.
pub type Inferred = Result<BTreeMap<String, canonical::Annotation>, Vec<solve::TypeError>>;

/// Each expression's type, by its region.
pub type ExprTypes = Vec<(TextRange, error::Type)>;

/// `module`'s annotations or type errors, as [`infer`] gives them, and
/// the type of each expression, by its region, in the order constraint
/// generation met them. Recording changes nothing about the solving.
pub fn infer_types(module: &crate::canonicalize::module::Canonical) -> (Inferred, ExprTypes) {
    let mut vars = Variables {
        recorded: Some(Vec::new()),
        ..Variables::default()
    };
    let constraint = constrain::module::constrain(&mut vars, module);
    let recorded = vars.recorded.take().unwrap_or_default();
    let result = solve::run(&mut vars, &constraint);
    let types = (recorded.into_iter())
        .map(|(region, ty)| {
            let var = vars.type_to_var(&ty);
            (region, vars.to_error_type(var))
        })
        .collect();
    (result, types)
}

/// The annotation of every top-level value of `module`, or its type
/// errors, the latest first.
pub fn infer(module: &crate::canonicalize::module::Canonical) -> Inferred {
    let mut vars = Variables::default();
    let constraint = constrain::module::constrain(&mut vars, module);
    solve::run(&mut vars, &constraint)
}

#[cfg(test)]
mod tests;
