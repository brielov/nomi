//! The formatter tree, mirroring elm-format's AST: `AST/V0_16.hs`,
//! `AST/Module.hs`, `AST/Listing.hs` and `ElmFormat/AST/Shared.hs`.
//!
//! Lowering builds it from the syntax tree, and
//! rendering reads it, so the two meet in upstream's terms. It keeps what
//! the output depends on: the comments each slot holds, the multiline
//! flags the source's line breaks set, and literals as values. It keeps no
//! spans; upstream renders without regions.
//!
//! Upstream's node kinds are separate types here, one per kind, and the
//! type-level labels on its comment slots (`'BeforeTerm`, `'AfterTerm`, …)
//! are field names and docs. Left out: the `ContainedCommentedList` and
//! `ExposedCommentedList` that no node holds.

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

impl Symbol {
    /// Whether this is `<|`, which puts what follows it below it.
    pub(crate) fn is_left_pipe(&self) -> bool {
        self.0 == "<|"
    }
}

/// A module name, or the qualifier before a name; empty when there is
/// none.
pub(crate) type Namespace = Vec<Upper>;

/// A qualified type or constructor name; upstream's `typeRef` and
/// `ctorRef`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UpperRef {
    pub(crate) namespace: Namespace,
    pub(crate) name: Upper,
}

/// A name an expression refers to; upstream's `varRef`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ref {
    Var(Namespace, Lower),
    Tag(Namespace, Upper),
    Op(Symbol),
}

/// The name a type annotation gives a type: upstream's `Ref ()`, which
/// its parser makes only of these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AnnotatedName {
    Var(Lower),
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

/// `value` with `comments` around it; upstream's `C`.
pub(crate) fn c<C, T>(comments: C, value: T) -> Commented<C, T> {
    Commented { comments, value }
}

impl<C: Default, T> Commented<C, T> {
    /// `value` with no comments around it.
    pub(crate) fn bare(value: T) -> Self {
        c(C::default(), value)
    }
}

/// The comments before a value and those after it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Around {
    pub(crate) before: Comments,
    pub(crate) after: Comments,
}

impl Around {
    pub(crate) fn new(before: Comments, after: Comments) -> Around {
        Around { before, after }
    }

    /// Whether no comment is on either side.
    pub(crate) fn is_empty(&self) -> bool {
        self.before.is_empty() && self.after.is_empty()
    }
}

/// The comments before a value, and the `--` comment ending its line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BeforeEol {
    pub(crate) before: Comments,
    pub(crate) eol: Option<String>,
}

/// The comments before and after a value, and the `--` comment ending
/// its line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AroundEol {
    pub(crate) before: Comments,
    pub(crate) after: Comments,
    pub(crate) eol: Option<String>,
}

/// The comments of an item after a separator: before the separator,
/// after it, and the `--` comment ending the item's line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SeparatorComments {
    pub(crate) before_separator: Comments,
    pub(crate) after_separator: Comments,
    pub(crate) eol: Option<String>,
}

/// Comments on one side.
pub(crate) type C1<T> = Commented<Comments, T>;
/// Comments before and after.
pub(crate) type C2<T> = Commented<Around, T>;
/// The `--` comment ending the value's line.
pub(crate) type C0Eol<T> = Commented<Option<String>, T>;
/// Comments before, and the `--` comment ending the value's line.
pub(crate) type C1Eol<T> = Commented<BeforeEol, T>;
/// Comments before and after, and the `--` comment ending the value's
/// line.
pub(crate) type C2Eol<T> = Commented<AroundEol, T>;

/// Items after separators, each with its comments. Only a leading
/// separator, which Elm lacks, would put comments before the first item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Sequence<T>(pub(crate) Vec<Commented<SeparatorComments, T>>);

/// A custom type's variants, without delimiters around them: all but the
/// last with comments before and after, the last with comments before;
/// upstream's `OpenCommentedList`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OpenCommentedList {
    pub(crate) items: Vec<C2Eol<NameWithArgs<Type>>>,
    pub(crate) last: C1Eol<NameWithArgs<Type>>,
}

/// Set when the source breaks a line inside the construct, which keeps
/// it broken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ForceMultiline(pub(crate) bool);

/// `key = value` or `key : value`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pair<V> {
    /// Comments after the key.
    pub(crate) key: C1<Lower>,
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

/// A type's name and its arguments, each with comments before it: the
/// parameters of a type it declares, or the types a variant holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NameWithArgs<A> {
    pub(crate) name: Upper,
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

/// A literal's value, decoded; rendering spells it anew.
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

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Expr {
    /// `()`, with the comments inside.
    Unit(Comments),
    Literal(LiteralValue),
    Var(Ref),
    App {
        function: Box<Expr>,
        /// Each with the comments before it.
        args: Vec<C1<Expr>>,
        multiline: ApplicationMultiline,
    },
    /// `-expr`; upstream's `Unary Negative`, as Elm has no other unary
    /// operator.
    Negate(Box<Expr>),
    /// The first operand, then at least one clause.
    Binops {
        first: Box<Expr>,
        clauses: Vec<BinopsClause>,
        multiline: ForceMultiline,
    },
    Parens(Box<C2<Expr>>),
    List {
        terms: Sequence<Expr>,
        /// Comments before `]`.
        trailing: Comments,
        force_multiline: ForceMultiline,
    },
    /// `[low..high]`, which Elm 0.19 rejects and elm-format still
    /// formats.
    Range {
        low: Box<C2<Expr>>,
        high: Box<C2<Expr>>,
    },
    /// Two or more elements.
    Tuple(Vec<C2<Expr>>, ForceMultiline),
    /// `(,)` and longer; the number of elements.
    TupleFunction(usize),
    Record {
        /// The record a `{ base | … }` updates.
        base: Option<C2<Lower>>,
        fields: Sequence<Pair<Expr>>,
        /// Comments before `}`.
        trailing: Comments,
        force_multiline: ForceMultiline,
    },
    /// `expr.field`.
    Access(Box<Expr>, Lower),
    /// `.field`.
    AccessFunction(Lower),
    Lambda {
        /// Each with the comments before it.
        args: Vec<C1<Pattern>>,
        /// The comments before `->` and after it.
        before_body: Comments,
        body: Box<Expr>,
        multiline: ForceMultiline,
    },
    If {
        first: Box<IfClause>,
        /// Each `else if`, with the comments before it.
        else_ifs: Vec<C1<IfClause>>,
        /// The `else` body, with the comments before it.
        else_body: Box<C1<Expr>>,
    },
    Let {
        declarations: Vec<LetDeclaration>,
        /// The comments after `in`.
        before_body: Comments,
        body: Box<Expr>,
    },
    Case {
        /// With the comments around it.
        subject: Box<C2<Expr>>,
        /// Whether `case … of` breaks.
        multiline: ForceMultiline,
        branches: Vec<CaseBranch>,
    },
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
    Construction {
        ctor: TypeConstructor,
        /// Each with the comments before it.
        args: Vec<C1<Type>>,
        force_multiline: ForceMultiline,
    },
    Parens(Box<C2<Type>>),
    /// Two or more elements.
    Tuple(Vec<C2Eol<Type>>, ForceMultiline),
    Record {
        /// The variable a `{ base | … }` extends.
        base: Option<C2<Lower>>,
        fields: Sequence<Pair<Type>>,
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
    /// `name args = body`.
    Definition {
        /// The name, or the pattern a destructuring binds.
        name: Pattern,
        /// Each with the comments before it.
        args: Vec<C1<Pattern>>,
        /// The comments before `=` and after it.
        before_body: Comments,
        body: Expr,
    },
    /// `name : type`.
    TypeAnnotation {
        /// With the comments after it.
        name: C1<AnnotatedName>,
        /// With the comments before it.
        ty: C1<Type>,
    },
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
        name_with_args: C2<NameWithArgs<Lower>>,
        variants: OpenCommentedList,
    },
    /// `type alias Name args = type`.
    TypeAlias {
        /// The comments between `type` and `alias`.
        before_alias: Comments,
        /// With the comments around it.
        name_with_args: C2<NameWithArgs<Lower>>,
        /// With the comments before it.
        ty: C1<Type>,
    },
    /// `port name : type`.
    PortAnnotation {
        /// With the comments around it.
        name: C2<Lower>,
        /// The comments after `:`.
        before_type: Comments,
        ty: Type,
    },
    /// Elm 0.16's `port name = expr`; upstream's
    /// `PortDefinition_until_0_16`.
    PortDefinition {
        /// With the comments around it.
        name: C2<Lower>,
        /// The comments after `=`.
        before_body: Comments,
        body: Expr,
    },
    /// `infix left 6 (+) = add`, each part with the comments before it.
    Fixity {
        assoc: C1<Assoc>,
        precedence: C1<i64>,
        /// With the comments after it too.
        op: C2<Symbol>,
        function: C1<Lower>,
    },
    /// Elm 0.18's `infixl 6 +`, whose operator is a symbol or a
    /// backticked function; upstream's `Fixity_until_0_18`.
    LegacyFixity {
        assoc: Assoc,
        before_precedence: Comments,
        precedence: i64,
        before_op: Comments,
        op: Symbol,
    },
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
pub(crate) struct Docs {
    pub(crate) blocks: Vec<markdown::Block<Option<ElmCode>>>,
    /// The comment as written, `{-|` to `-}`, for where reprinting the
    /// Markdown would end it elsewhere.
    pub(crate) source: String,
}

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
    Explicit(T, ForceMultiline),
    /// `(..)`, with the comments inside.
    Open(Around),
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

/// One of an effect module's `where { command = MyCmd, … }`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceSetting {
    pub(crate) key: C2<Lower>,
    pub(crate) value: C2<Upper>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Header {
    pub(crate) source_tag: SourceTag,
    pub(crate) name: C2<Namespace>,
    /// Comments before and after `where`'s settings.
    pub(crate) settings: Option<C2<Vec<SourceSetting>>>,
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

/// Where a module's imports of one module go: by name, then group, as
/// imports of one module stay apart where merging them would lose a
/// qualifier the module uses.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ImportKey {
    pub(crate) module: Namespace,
    pub(crate) group: usize,
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
    pub(crate) imports: C1<BTreeMap<ImportKey, C1<ImportMethod>>>,
    pub(crate) body: Vec<TopLevelStructure<Declaration>>,
}
