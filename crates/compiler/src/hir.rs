//! The HIR: a module as its source wrote it, upstream's `AST/Source.hs`
//! with IDs. Expressions, patterns, types and definitions live in arenas,
//! and the [`SourceMap`] takes each ID back to its CST node, so later
//! phases keep what they learn in tables keyed by ID and report at the
//! node. Names are as written: canonicalization says what they mean.
//!
//! Lowering is total. Where the CST is missing a piece, as in a module
//! with syntax errors the editor still asks about, the HIR has a
//! `Missing` node there; a module elm accepts has none.

mod lower;
mod shape;

use std::fmt;
use std::marker::PhantomData;
use std::ops::Index;

use nomi_base::TextRange;
use nomi_syntax::{SyntaxKind, SyntaxNode};

pub use lower::lower;
pub use shape::difference;

/// An index into an [`Arena`] of `T`.
pub struct Id<T> {
    raw: u32,
    kind: PhantomData<fn() -> T>,
}

impl<T> Id<T> {
    fn new(raw: usize) -> Id<T> {
        Id {
            raw: u32::try_from(raw).expect("fewer than 2^32 nodes"),
            kind: PhantomData,
        }
    }

    pub(crate) fn index(self) -> usize {
        self.raw as usize
    }
}

impl<T> Clone for Id<T> {
    fn clone(&self) -> Id<T> {
        *self
    }
}

impl<T> Copy for Id<T> {}

impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Id<T>) -> bool {
        self.raw == other.raw
    }
}

impl<T> Eq for Id<T> {}

impl<T> std::hash::Hash for Id<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.raw)
    }
}

/// An expression's ID.
pub type ExprId = Id<Expr>;
/// A pattern's ID.
pub type PatId = Id<Pattern>;
/// A type's ID.
pub type TypeId = Id<Type>;
/// A definition's ID.
pub type DefId = Id<Def>;

/// Values of one kind, each named by its [`Id`]. Flat, so a deep tree
/// drops without recursion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arena<T> {
    items: Vec<T>,
}

impl<T> Default for Arena<T> {
    fn default() -> Arena<T> {
        Arena { items: Vec::new() }
    }
}

impl<T> Arena<T> {
    fn alloc(&mut self, item: T) -> Id<T> {
        self.items.push(item);
        Id::new(self.items.len() - 1)
    }

    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    /// Each value, with its ID.
    pub fn iter(&self) -> impl Iterator<Item = (Id<T>, &T)> {
        self.items
            .iter()
            .enumerate()
            .map(|(i, item)| (Id::new(i), item))
    }
}

impl<T> Index<Id<T>> for Arena<T> {
    type Output = T;

    fn index(&self, id: Id<T>) -> &T {
        &self.items[id.index()]
    }
}

/// A CST node, by its kind and range, as rowan's `SyntaxNodePtr` names
/// one: it finds the node again in the tree it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodePtr {
    pub kind: SyntaxKind,
    pub range: TextRange,
}

impl NodePtr {
    fn new(node: &SyntaxNode) -> NodePtr {
        NodePtr {
            kind: node.kind(),
            range: node.text_range(),
        }
    }
}

/// Each ID's node; `None` for a `Missing` piece, which has none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceMap {
    pub(crate) exprs: Vec<Option<NodePtr>>,
    pub(crate) patterns: Vec<Option<NodePtr>>,
    pub(crate) types: Vec<Option<NodePtr>>,
    pub(crate) defs: Vec<Option<NodePtr>>,
}

impl SourceMap {
    /// The expression's range, leading trivia included; empty where it is
    /// `Missing`.
    pub(crate) fn expr_range(&self, id: ExprId) -> TextRange {
        range(self.exprs[id.index()])
    }

    /// As [`SourceMap::expr_range`], a pattern's.
    pub(crate) fn pattern_range(&self, id: PatId) -> TextRange {
        range(self.patterns[id.index()])
    }

    /// As [`SourceMap::expr_range`], a type's.
    pub(crate) fn type_range(&self, id: TypeId) -> TextRange {
        range(self.types[id.index()])
    }
}

fn range(node: Option<NodePtr>) -> TextRange {
    node.map(|p| p.range).unwrap_or_default()
}

/// A name as written, and where: `A.Located Name`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Spanned {
    pub text: String,
    pub range: TextRange,
}

/// An operator's associativity (`Binop.Associativity`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Associativity {
    Left,
    Non,
    Right,
}

/// How an operator groups with its neighbors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fixity {
    pub associativity: Associativity,
    pub precedence: i64,
}

/// A module: `Src.Module`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Module {
    /// The header's name; `Main` when there is no header.
    pub name: Option<Spanned>,
    pub kind: ModuleKind,
    pub exports: Exposing,
    /// The exposing list, from its `(` to its `)`.
    pub exports_range: TextRange,
    /// The module's doc comment, as written.
    pub docs: Option<Spanned>,
    /// Where the doc comment goes: from the end of the exposing list to
    /// the next token, as `chompModuleDocCommentSpace` locates its
    /// absence.
    pub docs_gap: TextRange,
    pub imports: Vec<Import>,
    /// The top-level values, in source order. Upstream's parser keeps
    /// values, unions, aliases and ports each in reverse source order
    /// (`categorizeDecls`), which canonicalization follows.
    pub values: Vec<DefId>,
    pub unions: Vec<Union>,
    pub aliases: Vec<Alias>,
    pub binops: Vec<Infix>,
    pub ports: Vec<Port>,
    pub exprs: Arena<Expr>,
    pub patterns: Arena<Pattern>,
    pub types: Arena<Type>,
    pub defs: Arena<Def>,
    pub source_map: SourceMap,
}

impl Module {
    /// The header's name, `Main` without one.
    pub fn name(&self) -> &str {
        self.name.as_ref().map_or("Main", |n| n.text.as_str())
    }

    /// The names of the top-level values, in source order.
    pub fn value_names(&self) -> impl DoubleEndedIterator<Item = &Spanned> {
        self.values.iter().filter_map(|&id| match &self.defs[id] {
            Def::Define { name, .. } => Some(name),
            Def::Destruct { .. } => None,
        })
    }
}

/// Which header the module has (`Src.Effects`, without the ports, which
/// are declarations).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ModuleKind {
    #[default]
    Plain,
    Port,
    Effect(ManagerFields),
}

/// An effect manager's `where { … }` fields (`Src.Manager`): the types its
/// commands and subscriptions are, the first of each key where elm's
/// parser would allow only one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManagerFields {
    /// The first field's key, which stands in for the whole clause.
    pub first: TextRange,
    pub command: Option<Spanned>,
    pub subscription: Option<Spanned>,
}

impl ManagerFields {
    /// Each field given, by its key.
    pub(crate) fn given(&self) -> impl Iterator<Item = (&'static str, &Spanned)> {
        [
            ("command", &self.command),
            ("subscription", &self.subscription),
        ]
        .into_iter()
        .filter_map(|(key, value)| Some((key, value.as_ref()?)))
    }
}

/// `Src.Exposing`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Exposing {
    /// `exposing (..)`, or no list at all.
    #[default]
    Open,
    Explicit(Vec<Exposed>),
}

/// `Src.Exposed`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exposed {
    Lower(Spanned),
    /// A type, with `(..)`'s range when its variants are exposed.
    Upper(Spanned, Option<TextRange>),
    Operator(Spanned),
}

impl Exposed {
    /// The name it exposes.
    pub fn name(&self) -> &Spanned {
        match self {
            Exposed::Lower(name) | Exposed::Upper(name, _) | Exposed::Operator(name) => name,
        }
    }
}

/// `Src.Import`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    pub name: Spanned,
    pub alias: Option<Spanned>,
    /// `None` when the import has no exposing list.
    pub exposing: Option<Exposing>,
}

/// `Src.Union`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Union {
    pub name: Spanned,
    pub params: Vec<Spanned>,
    pub variants: Vec<(Spanned, Vec<TypeId>)>,
    pub docs: Option<Spanned>,
    /// From its keyword to its end, without its doc comment, as upstream
    /// locates a declaration.
    pub range: TextRange,
}

/// `Src.Alias`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias {
    pub name: Spanned,
    pub params: Vec<Spanned>,
    pub body: TypeId,
    pub docs: Option<Spanned>,
    /// From its keyword to its end, without its doc comment, as upstream
    /// locates a declaration.
    pub range: TextRange,
}

/// `Src.Infix`: `infix left 6 (+) = add`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Infix {
    pub op: Spanned,
    pub fixity: Fixity,
    pub function: Spanned,
}

/// `Src.Port`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    pub name: Spanned,
    pub ty: TypeId,
    pub docs: Option<Spanned>,
    /// From its keyword to its end, without its doc comment, as upstream
    /// locates a declaration.
    pub range: TextRange,
}

/// `Src.Def`, and a top-level `Src.Value`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Def {
    Define {
        name: Spanned,
        params: Vec<PatId>,
        body: ExprId,
        annotation: Option<TypeId>,
        docs: Option<Spanned>,
    },
    Destruct {
        pattern: PatId,
        body: ExprId,
    },
}

/// `Src.Expr_`. Literals keep their text as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Missing,
    Chr(String),
    Str(String),
    Int(String),
    Float(String),
    /// A variable or constructor, maybe qualified: `Var` and `VarQual`.
    Var {
        qualifier: Option<String>,
        name: String,
        kind: VarType,
    },
    List(Vec<ExprId>),
    /// An operator as a function: `(+)`.
    Op(String),
    Negate(ExprId),
    /// A chain of operators, flat: `operands[0] ops[0] operands[1] …`.
    Binops {
        operands: Vec<ExprId>,
        ops: Vec<Spanned>,
    },
    Lambda {
        params: Vec<PatId>,
        body: ExprId,
    },
    Call {
        function: ExprId,
        args: Vec<ExprId>,
    },
    /// `if`s chained by `else if`, then the last `else`.
    If {
        branches: Vec<(ExprId, ExprId)>,
        otherwise: ExprId,
    },
    Let {
        defs: Vec<DefId>,
        body: ExprId,
    },
    Case {
        scrutinee: ExprId,
        branches: Vec<(PatId, ExprId)>,
    },
    Accessor(String),
    Access {
        record: ExprId,
        field: Spanned,
    },
    Update {
        record: Spanned,
        fields: Vec<(Spanned, ExprId)>,
    },
    Record(Vec<(Spanned, ExprId)>),
    Unit,
    /// Two or more.
    Tuple(Vec<ExprId>),
    /// The shader's source, between `[glsl|` and `|]`.
    Shader(String),
}

/// `Src.VarType`: whether a name is a variable's or a constructor's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarType {
    Low,
    Cap,
}

/// `Src.Pattern_`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pattern {
    Missing,
    Anything,
    Var(String),
    Record(Vec<Spanned>),
    Alias(PatId, Spanned),
    Unit,
    /// Two or more.
    Tuple(Vec<PatId>),
    /// A constructor, maybe qualified, with the range of its name and
    /// qualifier.
    Ctor {
        qualifier: Option<String>,
        name: Spanned,
        args: Vec<PatId>,
    },
    List(Vec<PatId>),
    Cons(PatId, PatId),
    Chr(String),
    Str(String),
    Int(String),
}

/// `Src.Type_`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Missing,
    Lambda(TypeId, TypeId),
    Var(String),
    /// A named type, maybe qualified, with the range of its name and
    /// qualifier.
    Named {
        qualifier: Option<String>,
        name: Spanned,
        args: Vec<TypeId>,
    },
    Record {
        fields: Vec<(Spanned, TypeId)>,
        extends: Option<Spanned>,
    },
    Unit,
    /// Two or more.
    Tuple(Vec<TypeId>),
}

/// Checks that `module`, lowered from `root`, keeps the source map's
/// promise: every ID's node is in the tree and of a kind that construct
/// lowers from, and nothing is `Missing`, as in a module elm accepts.
/// The problems, each a line.
pub fn verify(module: &Module, root: &SyntaxNode) -> Vec<String> {
    use nomi_syntax::ast::{self, AstNode};
    // Every node, found once: finding each from the root would cost the
    // tree's depth per ID.
    let nodes: std::collections::HashSet<NodePtr> =
        root.descendants().map(|n| NodePtr::new(&n)).collect();
    let mut problems = Vec::new();
    let mut check =
        |what: &str, i: usize, ptr: Option<NodePtr>, fits: fn(SyntaxKind) -> bool| match ptr {
            None => problems.push(format!("{what} {i} is missing")),
            Some(ptr) if !fits(ptr.kind) => {
                problems.push(format!("{what} {i} points at a {:?}", ptr.kind));
            }
            Some(ptr) if !nodes.contains(&ptr) => {
                problems.push(format!("{what} {i}'s {:?} isn't in the tree", ptr.kind));
            }
            Some(_) => {}
        };
    let map = &module.source_map;
    for (i, ptr) in map.exprs.iter().enumerate() {
        check("expression", i, *ptr, ast::Expr::can_cast);
    }
    for (i, ptr) in map.patterns.iter().enumerate() {
        check("pattern", i, *ptr, ast::Pattern::can_cast);
    }
    for (i, ptr) in map.types.iter().enumerate() {
        check("type", i, *ptr, ast::Type::can_cast);
    }
    for (i, ptr) in map.defs.iter().enumerate() {
        check("definition", i, *ptr, |kind| {
            matches!(
                kind,
                SyntaxKind::ValueDecl
                    | SyntaxKind::DestructureDecl
                    | SyntaxKind::LetValue
                    | SyntaxKind::LetDestructure
            )
        });
    }
    let missing = module
        .exprs
        .iter()
        .filter(|(_, e)| **e == Expr::Missing)
        .count()
        + module
            .patterns
            .iter()
            .filter(|(_, p)| **p == Pattern::Missing)
            .count()
        + module
            .types
            .iter()
            .filter(|(_, t)| **t == Type::Missing)
            .count();
    if missing > 0 {
        problems.push(format!("{missing} pieces are missing"));
    }
    let lengths = [
        (module.exprs.len(), map.exprs.len()),
        (module.patterns.len(), map.patterns.len()),
        (module.types.len(), map.types.len()),
        (module.defs.len(), map.defs.len()),
    ];
    if lengths.iter().any(|(a, b)| a != b) {
        problems.push(format!(
            "arenas and source map differ in length: {lengths:?}"
        ));
    }
    problems
}

/// An integer literal's value, maybe negative, wrapping at 64 bits as
/// elm's parser does.
pub(crate) fn int_value(text: &str) -> i64 {
    let (negative, text) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (digits, radix) = match text.strip_prefix("0x") {
        Some(hex) => (hex, 16),
        None => (text, 10),
    };
    let value = (digits.chars())
        .filter_map(|c| c.to_digit(radix))
        .fold(0i64, |n, d| {
            n.wrapping_mul(i64::from(radix)).wrapping_add(i64::from(d))
        });
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}

/// The HIR of the module `text`.
#[cfg(test)]
pub(crate) fn lowered(text: &str) -> Module {
    use nomi_syntax::ast::AstNode;
    let root = nomi_syntax::parse_module(text).syntax();
    lower(&nomi_syntax::ast::Module::cast(root).expect("a module"))
}

#[cfg(test)]
mod tests;
