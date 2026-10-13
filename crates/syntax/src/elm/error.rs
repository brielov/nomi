//! The errors of `Reporting/Error/Syntax.hs`, constructor for constructor.
//!
//! Each carries the position upstream gives it, where a report reads it:
//! where the problem is for the innermost, where the construct started
//! for those that wrap another.

#![allow(
    clippy::enum_variant_names,
    reason = "the names follow upstream's constructors"
)]

pub(super) use nomi_base::diagnostic::code::{Position, Region};

/// A name as written.
pub(super) type Name = Box<str>;

/// The column a `case` or `let` set its branches or definitions at.
pub(super) type Indent = u32;

/// `Error`: what parsing a module can fail with. The module name checks
/// are the build's, against the file's path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Ports in a plain `module` or an effect module: where its header
    /// says which it is.
    UnexpectedPort(Region),
    /// A `port module` with no ports: where its header says so.
    NoPorts(Region),
    /// A port in a package: the last port's name.
    NoPortsInPackage(Region),
    /// A `port module` in a package: where its header says so.
    NoPortModulesInPackage(Region),
    /// An `effect module` outside the kernel packages: where its header
    /// says so.
    NoEffectsOutsideKernel(Region),
    /// A module that doesn't parse.
    ParseError(Module),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Module {
    ModuleSpace(Space, Position),
    ModuleBadEnd(Position),
    ModuleProblem(Position),
    ModuleName(Position),
    ModuleExposing(Exposing, Position),
    PortModuleProblem(Position),
    PortModuleName(Position),
    PortModuleExposing(Exposing, Position),
    Effect(Position),
    FreshLine(Position),
    ImportStart(Position),
    ImportName(Position),
    ImportAs(Position),
    ImportAlias(Position),
    ImportExposing(Position),
    ImportExposingList(Exposing, Position),
    ImportEnd(Position),
    ImportIndentName(Position),
    ImportIndentAlias(Position),
    ImportIndentExposingList(Position),
    Infix(Position),
    Declarations(Decl),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exposing {
    ExposingSpace(Space, Position),
    ExposingStart(Position),
    ExposingValue(Position),
    ExposingOperator(Position),
    ExposingOperatorReserved(BadOperator, Position),
    ExposingOperatorRightParen(Position),
    ExposingTypePrivacy(Position),
    ExposingEnd(Position),
    ExposingIndentEnd(Position),
    ExposingIndentValue(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decl {
    DeclStart(Position),
    DeclSpace(Space, Position),
    Port(Port, Position),
    DeclType(DeclType, Position),
    DeclDef(Name, DeclDef, Position),
    DeclFreshLineAfterDocComment(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclDef {
    DeclDefSpace(Space, Position),
    DeclDefEquals(Position),
    DeclDefType(Type, Position),
    DeclDefArg(Pattern, Position),
    DeclDefBody(Expr, Position),
    DeclDefNameRepeat(Position),
    DeclDefNameMatch(Name, Position),
    DeclDefIndentType(Position),
    DeclDefIndentEquals(Position),
    DeclDefIndentBody(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Port {
    PortSpace(Space, Position),
    PortName(Position),
    PortColon(Position),
    PortType(Type, Position),
    PortIndentName(Position),
    PortIndentColon(Position),
    PortIndentType(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclType {
    DtSpace(Space, Position),
    DtName(Position),
    DtAlias(TypeAlias, Position),
    DtUnion(CustomType, Position),
    DtIndentName(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeAlias {
    AliasSpace(Space, Position),
    AliasName(Position),
    AliasEquals(Position),
    AliasBody(Type, Position),
    AliasIndentEquals(Position),
    AliasIndentBody(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CustomType {
    CtSpace(Space, Position),
    CtName(Position),
    CtEquals(Position),
    CtBar(Position),
    CtVariant(Position),
    CtVariantArg(Type, Position),
    CtIndentEquals(Position),
    CtIndentBar(Position),
    CtIndentAfterBar(Position),
    CtIndentAfterEquals(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Let(Box<Let>, Position),
    Case(Box<Case>, Position),
    If(Box<If>, Position),
    List(Box<List>, Position),
    Record(Box<Record>, Position),
    Tuple(Box<Tuple>, Position),
    Func(Box<Func>, Position),
    Dot(Position),
    Access(Position),
    OperatorRight(Name, Position),
    OperatorReserved(BadOperator, Position),
    Start(Position),
    Char(Char, Position),
    String(String, Position),
    Number(Number, Position),
    Space(Space, Position),
    /// 0.19.1's `EndlessShader`.
    ShaderEndless(Position),
    ShaderNotUtf8(Position),
    /// Parsec's message.
    ShaderProblem(Name, Position),
    IndentOperatorRight(Name, Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    RecordOpen(Position),
    RecordEnd(Position),
    RecordField(Position),
    RecordEquals(Position),
    RecordExpr(Expr, Position),
    RecordSpace(Space, Position),
    RecordIndentOpen(Position),
    RecordIndentEnd(Position),
    RecordIndentField(Position),
    RecordIndentEquals(Position),
    RecordIndentExpr(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tuple {
    TupleExpr(Expr, Position),
    TupleSpace(Space, Position),
    TupleEnd(Position),
    TupleOperatorClose(Position),
    TupleOperatorReserved(BadOperator, Position),
    TupleIndentExpr1(Position),
    TupleIndentExprN(Position),
    TupleIndentEnd(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum List {
    ListSpace(Space, Position),
    ListOpen(Position),
    ListExpr(Expr, Position),
    ListEnd(Position),
    ListIndentOpen(Position),
    ListIndentEnd(Position),
    ListIndentExpr(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Func {
    FuncSpace(Space, Position),
    FuncArg(Pattern, Position),
    FuncBody(Expr, Position),
    FuncArrow(Position),
    FuncIndentArg(Position),
    FuncIndentArrow(Position),
    FuncIndentBody(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Case {
    CaseSpace(Space, Position),
    CaseOf(Position),
    CasePattern(Pattern, Position),
    CaseArrow(Position),
    CaseExpr(Expr, Position),
    CaseBranch(Expr, Position),
    CaseIndentOf(Position),
    CaseIndentExpr(Position),
    CaseIndentPattern(Position),
    CaseIndentArrow(Position),
    CaseIndentBranch(Position),
    CasePatternAlignment(Indent, Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum If {
    IfSpace(Space, Position),
    IfThen(Position),
    IfElse(Position),
    IfElseBranchStart(Position),
    IfCondition(Expr, Position),
    IfThenBranch(Expr, Position),
    IfElseBranch(Expr, Position),
    IfIndentCondition(Position),
    IfIndentThen(Position),
    IfIndentThenBranch(Position),
    IfIndentElseBranch(Position),
    IfIndentElse(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Let {
    LetSpace(Space, Position),
    LetIn(Position),
    LetDefAlignment(Position),
    LetDefName(Position),
    LetDef(Name, Def, Position),
    LetDestruct(Destruct, Position),
    LetBody(Expr, Position),
    LetIndentDef(Position),
    LetIndentIn(Position),
    LetIndentBody(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Def {
    DefSpace(Space, Position),
    DefType(Type, Position),
    DefNameRepeat(Position),
    DefNameMatch(Name, Position),
    DefArg(Pattern, Position),
    DefEquals(Position),
    DefBody(Expr, Position),
    DefIndentEquals(Position),
    DefIndentType(Position),
    DefIndentBody(Position),
    DefAlignment(Indent, Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destruct {
    DestructSpace(Space, Position),
    DestructPattern(Pattern, Position),
    DestructEquals(Position),
    DestructBody(Expr, Position),
    DestructIndentEquals(Position),
    DestructIndentBody(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pattern {
    PRecord(PRecord, Position),
    PTuple(Box<PTuple>, Position),
    PList(Box<PList>, Position),
    PStart(Position),
    PChar(Char, Position),
    PString(String, Position),
    PNumber(Number, Position),
    /// The float's width.
    PFloat(u32, Position),
    PAlias(Position),
    PWildcardNotVar {
        name: Name,
        /// The width elm reports, which differs by release.
        width: u32,
        at: Position,
    },
    PSpace(Space, Position),
    PIndentStart(Position),
    PIndentAlias(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PRecord {
    PRecordOpen(Position),
    PRecordEnd(Position),
    PRecordField(Position),
    PRecordSpace(Space, Position),
    PRecordIndentOpen(Position),
    PRecordIndentEnd(Position),
    PRecordIndentField(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PTuple {
    PTupleOpen(Position),
    PTupleEnd(Position),
    PTupleExpr(Pattern, Position),
    PTupleSpace(Space, Position),
    PTupleIndentEnd(Position),
    PTupleIndentExpr1(Position),
    PTupleIndentExprN(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PList {
    PListOpen(Position),
    PListEnd(Position),
    PListExpr(Pattern, Position),
    PListSpace(Space, Position),
    PListIndentOpen(Position),
    PListIndentEnd(Position),
    PListIndentExpr(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    TRecord(Box<TRecord>, Position),
    TTuple(Box<TTuple>, Position),
    TStart(Position),
    TSpace(Space, Position),
    TIndentStart(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TRecord {
    TRecordOpen(Position),
    TRecordEnd(Position),
    TRecordField(Position),
    TRecordColon(Position),
    TRecordType(Type, Position),
    TRecordSpace(Space, Position),
    TRecordIndentOpen(Position),
    TRecordIndentField(Position),
    TRecordIndentColon(Position),
    TRecordIndentType(Position),
    TRecordIndentEnd(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TTuple {
    TTupleOpen(Position),
    TTupleEnd(Position),
    TTupleType(Type, Position),
    TTupleSpace(Space, Position),
    TTupleIndentType1(Position),
    TTupleIndentTypeN(Position),
    TTupleIndentEnd(Position),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Char {
    CharNotUtf8,
    CharEndless,
    CharEscape(Escape),
    /// The literal's width.
    CharNotString(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum String {
    StringNotUtf8,
    StringEndlessSingle,
    StringEndlessMulti,
    StringEscape(Escape),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Escape {
    EscapeUnknown,
    /// The widths are the escape's, from its backslash.
    BadUnicodeFormat(u32),
    BadUnicodeCode(u32),
    BadUnicodeLength {
        width: u32,
        digits: u32,
        /// The code point the digits spell.
        code: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Number {
    NumberEnd,
    /// The integer before the dot, as elm prints it: 0.19.1 reads it into
    /// a wrapping 64-bit integer.
    NumberDot(Name),
    NumberHexDigit,
    NumberNoLeadingZero,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    HasTab,
    HasNonUtf8,
    EndlessMultiComment,
}

/// `Parse/Symbol.hs`'s `BadOperator`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadOperator {
    BadDot,
    BadPipe,
    BadArrow,
    BadEquals,
    BadHasType,
}
