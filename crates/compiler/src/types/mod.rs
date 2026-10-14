//! Type inference (`Type/*.hs`), ported from 0.19.1's, which 0.19.2
//! keeps; 0.19.3's changes come in where they show. This module holds
//! the type variables, kept in a union-find store with what each stands
//! for, its rank and its mark, and the constraints inference solves
//! (`Type/Type.hs`).

mod constrain;
pub mod error;
pub mod expected;
mod instantiate;
pub mod localizer;
mod names;
mod occurs;
pub mod render;
pub mod report;
mod solve;
mod unify;
mod union_find;

use std::collections::BTreeMap;

use nomi_base::{Compat, Text, TextRange};

pub use solve::TypeError;

use crate::canonical::{self, Home};
use crate::canonicalize::module::Canonical;
use expected::{Category, Expected, PCategory, PExpected};
use union_find::{Point, UnionFind};

/// A type variable: a point in the [`Variables`] store.
type Variable = Point;

/// `Constraint`.
#[derive(Clone, Debug)]
enum Constraint {
    True,
    SaveTheEnvironment,
    Equal {
        region: TextRange,
        category: Category,
        ty: Type,
        expected: Expected<Type>,
    },
    Local {
        region: TextRange,
        name: String,
        expected: Expected<Type>,
    },
    Foreign {
        region: TextRange,
        name: String,
        annotation: Box<canonical::Annotation>,
        expected: Expected<Type>,
    },
    Pattern {
        region: TextRange,
        category: PCategory,
        ty: Type,
        expected: PExpected<Type>,
    },
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
fn exists(flex_vars: Vec<Variable>, constraint: Constraint) -> Constraint {
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
enum FlatType {
    App1(Home, Text, Vec<Variable>),
    Fun1(Variable, Variable),
    EmptyRecord1,
    Record1(BTreeMap<Text, Variable>, Variable),
    Unit1,
    Tuple1(Variable, Variable, Option<Variable>),
}

/// `Type`: a type as constraints write it, parts of it variables.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Type {
    Alias(Home, Text, Vec<(String, Type)>, Box<Type>),
    Var(Variable),
    App(Home, Text, Vec<Type>),
    Fun(Box<Type>, Box<Type>),
    EmptyRecord,
    Record(BTreeMap<Text, Type>, Box<Type>),
    Unit,
    Tuple(Box<Type>, Box<Type>, Option<Box<Type>>),
}

/// `==>`: a function type.
fn fun(arg: Type, result: Type) -> Type {
    Type::Fun(Box::new(arg), Box::new(result))
}

/// `Descriptor`: what a variable stands for, the rank of the `let` that
/// binds it, a mark for traversals, and its copy while instantiating.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Descriptor {
    content: Content,
    rank: u32,
    mark: Mark,
    copy: Option<Variable>,
}

impl Descriptor {
    /// `makeDescriptor`.
    fn new(content: Content) -> Descriptor {
        Descriptor::at_rank(content, NO_RANK)
    }

    /// A fresh variable's descriptor at `rank`.
    fn at_rank(content: Content, rank: u32) -> Descriptor {
        Descriptor {
            content,
            rank,
            mark: NO_MARK,
            copy: None,
        }
    }
}

/// `Content`.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Content {
    FlexVar(Option<String>),
    FlexSuper(SuperType, Option<String>),
    RigidVar(String),
    RigidSuper(SuperType, String),
    Structure(FlatType),
    Alias(Home, Text, Vec<(String, Variable)>, Variable),
    Error,
}

impl Content {
    /// `nameToFlex`'s content: flexible, named `name`, and ranging over
    /// what the name says.
    fn flex_named(name: &str) -> Content {
        match SuperType::of_name(name) {
            Some(super_type) => Content::FlexSuper(super_type, Some(name.to_owned())),
            None => Content::FlexVar(Some(name.to_owned())),
        }
    }

    /// `nameToRigid`'s content.
    fn rigid_named(name: &str) -> Content {
        match SuperType::of_name(name) {
            Some(super_type) => Content::RigidSuper(super_type, name.to_owned()),
            None => Content::RigidVar(name.to_owned()),
        }
    }
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
    pub(crate) fn of_name(name: &str) -> Option<SuperType> {
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
}

/// Whether [`children`] takes an alias's real type.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AliasReal {
    Include,
    Skip,
}

/// The variables `content` holds, in order: an alias's arguments, then
/// its real type where `real` takes it.
fn children(content: &Content, real: AliasReal) -> Vec<Variable> {
    match content {
        Content::FlexVar(_)
        | Content::FlexSuper(..)
        | Content::RigidVar(_)
        | Content::RigidSuper(..)
        | Content::Error => Vec::new(),
        Content::Structure(flat) => match flat {
            FlatType::App1(_, _, args) => args.clone(),
            FlatType::Fun1(a, b) => vec![*a, *b],
            FlatType::EmptyRecord1 | FlatType::Unit1 => Vec::new(),
            FlatType::Record1(fields, ext) => fields.values().copied().chain([*ext]).collect(),
            FlatType::Tuple1(a, b, c) => [Some(*a), Some(*b), *c].into_iter().flatten().collect(),
        },
        Content::Alias(_, _, args, alias_real) => {
            let real = (real == AliasReal::Include).then_some(*alias_real);
            args.iter().map(|(_, a)| *a).chain(real).collect()
        }
    }
}

/// `noRank`: a variable no `let` has bound yet.
const NO_RANK: u32 = 0;
/// `outermostRank`.
const OUTERMOST_RANK: u32 = 1;

/// `Mark`: what a traversal has visited.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Mark(u32);

/// `getVarNamesMark`.
const GET_VAR_NAMES_MARK: Mark = Mark(0);
/// `occursMark`.
const OCCURS_MARK: Mark = Mark(1);
/// `noMark`, which every fresh variable has.
const NO_MARK: Mark = Mark(2);

impl Mark {
    /// `nextMark`.
    fn next(self) -> Mark {
        Mark(self.0 + 1)
    }
}

/// Whether `name` of `home` is elm/core's `module` type `type_name`.
fn is_core(home: &Home, name: &str, module: &str, type_name: &str) -> bool {
    name == type_name && home.is("elm/core", module)
}

fn is_int(home: &Home, name: &str) -> bool {
    is_core(home, name, "Basics", "Int")
}

fn is_float(home: &Home, name: &str) -> bool {
    is_core(home, name, "Basics", "Float")
}

fn is_bool(home: &Home, name: &str) -> bool {
    is_core(home, name, "Basics", "Bool")
}

fn is_string(home: &Home, name: &str) -> bool {
    is_core(home, name, "String", "String")
}

fn is_char(home: &Home, name: &str) -> bool {
    is_core(home, name, "Char", "Char")
}

fn is_list(home: &Home, name: &str) -> bool {
    is_core(home, name, "List", "List")
}

fn is_maybe(home: &Home, name: &str) -> bool {
    is_core(home, name, "Maybe", "Maybe")
}

/// elm/core's type `name`, from its module `module`, with no arguments.
fn core_type(module: &str, name: &str) -> Type {
    Type::App(Home::core(module), Text::from(name), Vec::new())
}

fn int() -> Type {
    core_type("Basics", "Int")
}

fn float() -> Type {
    core_type("Basics", "Float")
}

fn bool() -> Type {
    core_type("Basics", "Bool")
}

fn never() -> Type {
    core_type("Basics", "Never")
}

fn char() -> Type {
    core_type("Char", "Char")
}

fn string() -> Type {
    core_type("String", "String")
}

/// `List entry`.
fn list(entry: Type) -> Type {
    Type::App(Home::core("List"), Text::from("List"), vec![entry])
}

fn package_type(package: &str, module: &str, name: &str) -> Type {
    let home = Home {
        package: package.into(),
        module: module.into(),
    };
    Type::App(home, Text::from(name), Vec::new())
}

/// WebGL's types, which shaders' attributes, uniforms and varyings have.
fn vec2() -> Type {
    package_type("elm-explorations/linear-algebra", "Math.Vector2", "Vec2")
}

fn vec3() -> Type {
    package_type("elm-explorations/linear-algebra", "Math.Vector3", "Vec3")
}

fn vec4() -> Type {
    package_type("elm-explorations/linear-algebra", "Math.Vector4", "Vec4")
}

fn mat4() -> Type {
    package_type("elm-explorations/linear-algebra", "Math.Matrix4", "Mat4")
}

fn texture() -> Type {
    package_type("elm-explorations/webgl", "WebGL.Texture", "Texture")
}

/// The type variables of one inference, each with its descriptor.
#[derive(Clone, Debug)]
struct Variables {
    store: UnionFind<Descriptor>,
    /// The release whose name order shows where a record's fields are
    /// walked.
    compat: Compat,
    /// Where constraint generation records each expression's type, when
    /// asked to: its region and the type it is expected to
    /// have, which solving makes its type.
    recorded: Option<Vec<(TextRange, Type)>>,
}

impl Default for Variables {
    fn default() -> Variables {
        Variables::new(Compat::V0_19_1)
    }
}

/// `map`'s entries in `compat`'s name order, as upstream's maps keep
/// them: 0.19.3 compares names length first.
pub(crate) fn in_name_order<K: AsRef<str>, T>(
    compat: Compat,
    map: &BTreeMap<K, T>,
) -> Vec<(&K, &T)> {
    let mut entries: Vec<(&K, &T)> = map.iter().collect();
    if compat == Compat::V0_19_3 {
        entries.sort_by(|a, b| compat.compare_names(a.0.as_ref(), b.0.as_ref()));
    }
    entries
}

impl Variables {
    /// The variables of an inference as `compat`.
    fn new(compat: Compat) -> Variables {
        Variables {
            store: UnionFind::default(),
            compat,
            recorded: None,
        }
    }

    /// `UF.fresh` of `content`, unranked.
    fn fresh(&mut self, content: Content) -> Variable {
        self.store.fresh(Descriptor::new(content))
    }

    /// `UF.fresh` of a whole descriptor.
    fn fresh_descriptor(&mut self, descriptor: Descriptor) -> Variable {
        self.store.fresh(descriptor)
    }

    /// `mkFlexVar`.
    fn flex_var(&mut self) -> Variable {
        self.fresh(Content::FlexVar(None))
    }

    /// `mkFlexNumber`.
    fn flex_number(&mut self) -> Variable {
        self.fresh(Content::FlexSuper(SuperType::Number, None))
    }

    /// `nameToFlex`.
    fn name_to_flex(&mut self, name: &str) -> Variable {
        self.fresh(Content::flex_named(name))
    }

    /// `nameToRigid`.
    fn name_to_rigid(&mut self, name: &str) -> Variable {
        self.fresh(Content::rigid_named(name))
    }

    fn get(&mut self, var: Variable) -> &Descriptor {
        self.store.get(var)
    }

    fn get_mut(&mut self, var: Variable) -> &mut Descriptor {
        self.store.get_mut(var)
    }

    /// What `var` stands for, copied out of the store.
    fn content(&mut self, var: Variable) -> Content {
        self.get(var).content.clone()
    }

    fn set(&mut self, var: Variable, descriptor: Descriptor) {
        self.store.set(var, descriptor);
    }

    fn union(&mut self, a: Variable, b: Variable, descriptor: Descriptor) {
        self.store.union(a, b, descriptor);
    }

    fn equivalent(&mut self, a: Variable, b: Variable) -> bool {
        self.store.equivalent(a, b)
    }

    fn redundant(&self, var: Variable) -> bool {
        self.store.redundant(var)
    }

    /// A variable standing for `ty`, each variable for a part of it made
    /// by `make`, as the solver pools them or, once solving is done, not.
    fn type_to_var(
        &mut self,
        ty: &Type,
        make: &mut impl FnMut(&mut Variables, Content) -> Variable,
    ) -> Variable {
        nomi_base::grow(|| match ty {
            Type::Var(v) => *v,
            Type::App(home, name, args) => {
                let args = args.iter().map(|a| self.type_to_var(a, make)).collect();
                let flat = FlatType::App1(home.clone(), name.clone(), args);
                make(self, Content::Structure(flat))
            }
            Type::Fun(a, b) => {
                let a = self.type_to_var(a, make);
                let b = self.type_to_var(b, make);
                make(self, Content::Structure(FlatType::Fun1(a, b)))
            }
            Type::Alias(home, name, args, real) => {
                let args = (args.iter())
                    .map(|(n, a)| (n.clone(), self.type_to_var(a, make)))
                    .collect();
                let real = self.type_to_var(real, make);
                make(self, Content::Alias(home.clone(), name.clone(), args, real))
            }
            Type::Record(fields, ext) => {
                let fields = (fields.iter())
                    .map(|(n, f)| (n.clone(), self.type_to_var(f, make)))
                    .collect();
                let ext = self.type_to_var(ext, make);
                make(self, Content::Structure(FlatType::Record1(fields, ext)))
            }
            Type::EmptyRecord => make(self, Content::Structure(FlatType::EmptyRecord1)),
            Type::Unit => make(self, Content::Structure(FlatType::Unit1)),
            Type::Tuple(a, b, c) => {
                let a = self.type_to_var(a, make);
                let b = self.type_to_var(b, make);
                let c = c.as_ref().map(|c| self.type_to_var(c, make));
                make(self, Content::Structure(FlatType::Tuple1(a, b, c)))
            }
        })
    }
}

/// A module's top-level annotations, or its type errors, the latest
/// first.
pub type Inferred = Result<BTreeMap<String, canonical::Annotation>, Vec<TypeError>>;

/// Each expression's and pattern's type, by its region.
pub type ExprTypes = Vec<(TextRange, error::Type)>;

/// `module`'s annotations or type errors, as [`infer`] gives them, and
/// the type of each expression, by its region, in the order constraint
/// generation met them. Recording changes nothing about the solving.
pub fn infer_types(compat: Compat, module: &Canonical) -> (Inferred, ExprTypes) {
    let vars = Variables {
        recorded: Some(Vec::new()),
        ..Variables::new(compat)
    };
    let (result, Leftovers { mut vars, .. }) = run(vars, module);
    let recorded = vars.recorded.take().unwrap_or_default();
    let types = (recorded.into_iter())
        .map(|(region, ty)| {
            let var = vars.type_to_var(&ty, &mut Variables::fresh);
            (region, names::to_error_type(&mut vars, var))
        })
        .collect();
    (result, types)
}

/// The annotation of every top-level value of `module`, or its type
/// errors, the latest first.
pub fn infer(compat: Compat, module: &Canonical) -> Inferred {
    infer_leaving(compat, module).0
}

/// What an inference leaves behind: its variables and constraints, a
/// large module's millions of them, for the caller to free where it
/// costs least.
pub struct Leftovers {
    vars: Variables,
    _constraint: Constraint,
}

impl std::fmt::Debug for Leftovers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Leftovers")
    }
}

/// [`infer`], and what it leaves behind.
pub fn infer_leaving(compat: Compat, module: &Canonical) -> (Inferred, Leftovers) {
    run(Variables::new(compat), module)
}

/// `module` constrained and solved with `vars`.
fn run(mut vars: Variables, module: &Canonical) -> (Inferred, Leftovers) {
    let constraint = constrain::module::constrain(&mut vars, module);
    let inferred = solve::run(&mut vars, &constraint);
    let leftovers = Leftovers {
        vars,
        _constraint: constraint,
    };
    (inferred, leftovers)
}

#[cfg(test)]
mod tests;
