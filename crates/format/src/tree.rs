//! The formatter tree, mirroring elm-format's AST: `AST/V0_16.hs`,
//! `AST/Module.hs`, `AST/Listing.hs` and `ElmFormat/AST/Shared.hs`.
//!
//! Lowering (plan 2.4 to 2.7) builds it from the syntax tree, and
//! rendering reads it, so the two meet in upstream's terms. It keeps what
//! the output depends on: the comments each slot holds, the multiline
//! flags the source's line breaks set, and literals as values. It keeps no
//! spans; upstream renders without regions.
//!
//! Upstream's node kinds are separate types here, one per kind, and the
//! type-level labels on its comment slots (`'BeforeTerm`, `'AfterTerm`, …)
//! are field docs. Left out: the `ContainedCommentedList` and `ExposedCommentedList` that no node holds.

use std::collections::BTreeMap;

use crate::markdown;

/// `LowercaseIdentifier`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Lower(pub(crate) String);

/// `UppercaseIdentifier`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Upper(pub(crate) String);

/// `SymbolIdentifier`: an operator, without its parentheses.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Symbol(pub(crate) String);

/// A module name, or the qualifier before a name; empty when there is
/// none.
pub(crate) type Namespace = Vec<Upper>;

/// A qualified type or constructor name; upstream's `typeRef` and
/// `ctorRef`.
pub(crate) type UpperRef = (Namespace, Upper);

/// A name an expression refers to; upstream's `varRef` is `Ref Namespace`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ref<Ns = Namespace> {
    Var(Ns, Lower),
    Tag(Ns, Upper),
    Op(Symbol),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Comment {
    /// `{- … -}`, one string per line, without the delimiters.
    Block(Vec<String>),
    /// `-- …`, without the dashes.
    Line(String),
    /// `{--}`.
    TrickOpener,
    /// `--}`.
    TrickCloser,
    /// `{--…-}`, without the delimiters.
    TrickBlock(String),
}

pub(crate) type Comments = Vec<Comment>;

/// A value with comments around it; upstream's `C`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Commented<C, T> {
    pub(crate) comments: C,
    pub(crate) value: T,
}

/// Comments on one side.
pub(crate) type C1<T> = Commented<Comments, T>;
/// Comments before and after.
pub(crate) type C2<T> = Commented<(Comments, Comments), T>;
/// The `--` comment ending the value's line.
pub(crate) type C0Eol<T> = Commented<Option<String>, T>;
/// Comments on one side, and the `--` comment ending the value's line.
pub(crate) type C1Eol<T> = Commented<(Comments, Option<String>), T>;
/// Comments before and after, and the `--` comment ending the value's
/// line.
pub(crate) type C2Eol<T> = Commented<(Comments, Comments, Option<String>), T>;

/// Items after separators, each with the comments before its separator,
/// after it, and ending its line. Only a leading separator, which Elm
/// lacks, would put comments before the first item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Sequence<T>(pub(crate) Vec<C2Eol<T>>);

/// Items without delimiters around them, like a custom type's variants:
/// all but the last with comments before and after, the last with
/// comments before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpenCommentedList<T> {
    pub(crate) items: Vec<C2Eol<T>>,
    pub(crate) last: C1Eol<T>,
}

/// Set when the source breaks a line inside the construct, which keeps
/// it broken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ForceMultiline(pub(crate) bool);

/// `key = value` or `key : value`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pair<K, V> {
    /// Comments after the key.
    pub(crate) key: C1<K>,
    /// Comments before the value.
    pub(crate) value: C1<V>,
    pub(crate) force_multiline: ForceMultiline,
}

/// Whether a construct's parts go on one line or one per line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Multiline {
    JoinAll,
    SplitAll,
}

/// How a function application's lines break; upstream's
/// `FunctionApplicationMultiline`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApplicationMultiline {
    /// The function and every argument on its own line.
    SplitFirst,
    /// The first argument on the function's line; the rest joined to it
    /// too with `JoinAll`.
    JoinFirst(Multiline),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Assoc {
    Left,
    Non,
    Right,
}

/// A name and its arguments, each with comments before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NameWithArgs<N, A> {
    pub(crate) name: N,
    pub(crate) args: Vec<C1<A>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TypeConstructor {
    Named(UpperRef),
    /// `(,)` and longer; the number of elements, at least 2.
    Tuple(usize),
}

/// `op expr` after the first operand, with the comments before and after
/// the operator.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BinopsClause {
    pub(crate) before_op: Comments,
    pub(crate) op: Ref,
    pub(crate) after_op: Comments,
    pub(crate) expr: Expr,
}

/// `if condition then body`, each with comments around it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct IfClause {
    pub(crate) condition: C2<Expr>,
    pub(crate) body: C2<Expr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntRepr {
    Decimal,
    Hexadecimal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FloatRepr {
    Decimal,
    Exponent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StringRepr {
    SingleQuoted,
    TripleQuoted,
}

/// A literal's value, decoded; rendering spells it anew (plan 2.12).
///
/// Characters are code points as elm-format holds them, which a Rust
/// `char` can't: escapes give lone surrogates, and `\x` larger values.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LiteralValue {
    Int(i64, IntRepr),
    Float(f64, FloatRepr),
    Char(u32),
    String(Vec<u32>, StringRepr),
    Bool(bool),
}

/// Upstream's `UnaryOperator`; Elm has only negation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnaryOperator {
    Negative,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Expr {
    /// `()`, with the comments inside.
    Unit(Comments),
    Literal(LiteralValue),
    Var(Ref),
    /// The function, its arguments with comments before each, and how it
    /// breaks.
    App(Box<Expr>, Vec<C1<Expr>>, ApplicationMultiline),
    Unary(UnaryOperator, Box<Expr>),
    /// The first operand, then at least one clause; multiline.
    Binops(Box<Expr>, Vec<BinopsClause>, bool),
    Parens(Box<C2<Expr>>),
    List {
        terms: Sequence<Expr>,
        /// Comments before `]`.
        trailing: Comments,
        force_multiline: ForceMultiline,
    },
    /// `[a..b]`, which Elm 0.19 rejects and elm-format still formats;
    /// multiline.
    Range(Box<C2<Expr>>, Box<C2<Expr>>, bool),
    /// Two or more elements; multiline.
    Tuple(Vec<C2<Expr>>, bool),
    /// `(,)` and longer; the number of elements.
    TupleFunction(usize),
    Record {
        /// The record a `{ base | … }` updates.
        base: Option<C2<Lower>>,
        fields: Sequence<Pair<Lower, Expr>>,
        /// Comments before `}`.
        trailing: Comments,
        force_multiline: ForceMultiline,
    },
    /// `expr.field`.
    Access(Box<Expr>, Lower),
    /// `.field`.
    AccessFunction(Lower),
    /// The arguments with comments before each, comments before `->`, the
    /// body, and multiline.
    Lambda(Vec<C1<Pattern>>, Comments, Box<Expr>, bool),
    /// The first clause, each `else if` with comments before it, and the
    /// `else` body with comments before it.
    If(Box<IfClause>, Vec<C1<IfClause>>, Box<C1<Expr>>),
    /// The declarations, comments after `in`, and the body.
    Let(Vec<LetDeclaration>, Comments, Box<Expr>),
    /// The subject with comments around it, whether `case … of` breaks,
    /// and the branches.
    Case(Box<C2<Expr>>, bool, Vec<CaseBranch>),
    /// `[glsl| … |]`, the source between the delimiters.
    GlShader(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CaseBranch {
    pub(crate) before_pattern: Comments,
    pub(crate) before_arrow: Comments,
    pub(crate) after_arrow: Comments,
    pub(crate) pattern: Pattern,
    pub(crate) body: Expr,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Pattern {
    /// `_`.
    Anything,
    /// `()`, with the comments inside.
    Unit(Comments),
    Literal(LiteralValue),
    Var(Lower),
    /// `(+)`, as a definition's name.
    Op(Symbol),
    /// A constructor and its arguments, with comments before each.
    Data(UpperRef, Vec<C1<Pattern>>),
    Parens(Box<C2<Pattern>>),
    /// Two or more elements.
    Tuple(Vec<C2<Pattern>>),
    /// `[]`, with the comments inside.
    EmptyList(Comments),
    List(Vec<C2<Pattern>>),
    /// `first :: rest`.
    Cons {
        first: Box<C0Eol<Pattern>>,
        rest: Sequence<Pattern>,
    },
    /// `{}`, with the comments inside.
    EmptyRecord(Comments),
    Record(Vec<C2<Lower>>),
    /// `pattern as name`, with comments after the pattern and before the
    /// name.
    Alias(Box<C1<Pattern>>, C1<Lower>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Type {
    /// `()`, with the comments inside.
    Unit(Comments),
    Variable(Lower),
    /// A constructor and its arguments, with comments before each.
    Construction(TypeConstructor, Vec<C1<Type>>, ForceMultiline),
    Parens(Box<C2<Type>>),
    /// Two or more elements.
    Tuple(Vec<C2Eol<Type>>, ForceMultiline),
    Record {
        /// The variable a `{ base | … }` extends.
        base: Option<C2<Lower>>,
        fields: Sequence<Pair<Lower, Type>>,
        /// Comments before `}`.
        trailing: Comments,
        force_multiline: ForceMultiline,
    },
    /// `first -> rest…`.
    Function {
        first: Box<C0Eol<Type>>,
        rest: Sequence<Type>,
        force_multiline: ForceMultiline,
    },
}

/// What may appear both at the top level and in a `let`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CommonDeclaration {
    /// The name or pattern, the arguments with comments before each,
    /// comments before `=`, and the body.
    Definition(Pattern, Vec<C1<Pattern>>, Comments, Expr),
    /// The name with comments after it, and the type with comments before
    /// it.
    TypeAnnotation(C1<Ref<()>>, C1<Type>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LetDeclaration {
    Common(Box<CommonDeclaration>),
    Comment(Comment),
}

/// A top-level declaration.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Declaration {
    Common(CommonDeclaration),
    /// `type Name args = variants`.
    Datatype {
        name_with_args: C2<NameWithArgs<Upper, Lower>>,
        variants: OpenCommentedList<NameWithArgs<Upper, Type>>,
    },
    /// Comments after `alias`, the name and arguments with comments
    /// around them, and the type with comments before it.
    TypeAlias(Comments, C2<NameWithArgs<Upper, Lower>>, C1<Type>),
    /// The name with comments around it, comments after `:`, and the type.
    PortAnnotation(C2<Lower>, Comments, Type),
    /// Elm 0.16's `port name = expr`: the name with comments around it,
    /// comments after `=`, and the value; upstream's
    /// `PortDefinition_until_0_16`.
    PortDefinition(C2<Lower>, Comments, Expr),
    /// `infix left 6 (+) = add`, each part with comments before it, and
    /// after the operator.
    Fixity(C1<Assoc>, C1<i64>, C2<Symbol>, C1<Lower>),
    /// Elm 0.18's `infixl 6 +`, with comments before the precedence and
    /// before the operator, a symbol or a backticked function; upstream's
    /// `Fixity_until_0_18`.
    LegacyFixity(Assoc, Comments, i64, Comments, Symbol),
}

/// An entry at the top level, or a comment between them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TopLevelStructure<T> {
    DocComment(Docs),
    BodyComment(Comment),
    Entry(T),
}

/// A doc comment's Markdown, with the Elm code of its code blocks where
/// elm-format reads some.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Docs(pub(crate) Vec<markdown::Block<Option<ElmCode>>>);

/// The Elm in a code block, read as the first of these that parses;
/// upstream's `ElmCodeBlock`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ElmCode {
    Declarations(Vec<TopLevelStructure<Declaration>>),
    Expressions(Vec<TopLevelStructure<C0Eol<Expr>>>),
    Module(Box<Module>),
}

/// Comments before and after each key, in order; upstream's
/// `CommentedMap`. Keys sort as upstream's `Ord` does: by code point.
pub(crate) type CommentedMap<K, V> = BTreeMap<K, C2<V>>;

/// An `exposing` list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Listing<T> {
    /// The items, and whether the source breaks the list.
    Explicit(T, bool),
    /// `(..)`, with the comments inside.
    Open(Comments, Comments),
    /// Nothing exposed, as with an `import` that has no `exposing`.
    Closed,
}

/// The items of an `exposing` list, each kind in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DetailedListing {
    pub(crate) values: CommentedMap<Lower, ()>,
    pub(crate) operators: CommentedMap<Symbol, ()>,
    /// Each type, with its constructors' listing and the comments before
    /// it.
    pub(crate) types: CommentedMap<Upper, C1<Listing<CommentedMap<Upper, ()>>>>,
}

/// One item of an `exposing` list, as rendering sorts them; upstream's
/// `Listing.Value`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    Var(Lower),
    Op(Symbol),
    /// A type with comments after its name, and its constructors.
    Union(C1<Upper>, Listing<CommentedMap<Upper, ()>>),
}

/// `module` (`Normal`), `effect module` or `port module`, with the
/// comments after the keyword.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceTag {
    Normal,
    Effect(Comments),
    Port(Comments),
}

/// An effect module's `where { command = MyCmd, … }`.
pub(crate) type SourceSettings = Vec<(C2<Lower>, C2<Upper>)>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Header {
    pub(crate) source_tag: SourceTag,
    pub(crate) name: C2<Namespace>,
    /// Comments before and after `where`'s settings.
    pub(crate) settings: Option<C2<SourceSettings>>,
    /// Comments before and after `exposing`.
    pub(crate) exports: Option<C2<Listing<DetailedListing>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImportMethod {
    /// Comments before and after `as`.
    pub(crate) alias: Option<C2<Upper>>,
    /// Comments before and after `exposing`.
    pub(crate) exposed: C2<Listing<DetailedListing>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Module {
    /// Comments before the header.
    pub(crate) initial_comments: Comments,
    /// `None` for a module without `module … exposing`.
    pub(crate) header: Option<Header>,
    /// The module's doc comment.
    pub(crate) docs: Option<Docs>,
    /// One per module, merged and sorted, with comments before each, and
    /// comments before them all.
    pub(crate) imports: C1<BTreeMap<Namespace, C1<ImportMethod>>>,
    pub(crate) body: Vec<TopLevelStructure<Declaration>>,
}
