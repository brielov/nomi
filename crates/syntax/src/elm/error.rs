//! The errors of `Reporting/Error/Syntax.hs`, constructor for constructor.
//!
//! Each carries the cursor upstream gives it: where the problem is for the
//! innermost, where the construct started for those that wrap another.

#![allow(
    clippy::enum_variant_names,
    reason = "the names follow upstream's constructors"
)]

/// A position as upstream keeps it: 0.19.1 counts rows and columns from 1
/// in 16 bits, later releases from 0 in 32.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cursor {
    pub row: u32,
    pub col: u32,
}

/// A span of the module, from cursor to cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Region {
    pub start: Cursor,
    pub end: Cursor,
}

/// A name as written.
pub type Name = Box<str>;

/// The column a `case` or `let` set its branches or definitions at.
pub type Indent = u32;

/// `Error`: what parsing a module can fail with. The module name checks
/// are the build's, against the file's path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    UnexpectedPort(Region),
    NoPorts(Region),
    NoPortsInPackage(Name, Region),
    NoPortModulesInPackage(Region),
    NoEffectsOutsideKernel(Region),
    ParseError(Module),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Module {
    ModuleSpace(Space, Cursor),
    ModuleBadEnd(Cursor),
    ModuleProblem(Cursor),
    ModuleName(Cursor),
    ModuleExposing(Exposing, Cursor),
    PortModuleProblem(Cursor),
    PortModuleName(Cursor),
    PortModuleExposing(Exposing, Cursor),
    Effect(Cursor),
    FreshLine(Cursor),
    ImportStart(Cursor),
    ImportName(Cursor),
    ImportAs(Cursor),
    ImportAlias(Cursor),
    ImportExposing(Cursor),
    ImportExposingList(Exposing, Cursor),
    ImportEnd(Cursor),
    ImportIndentName(Cursor),
    ImportIndentAlias(Cursor),
    ImportIndentExposingList(Cursor),
    Infix(Cursor),
    Declarations(Decl, Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exposing {
    ExposingSpace(Space, Cursor),
    ExposingStart(Cursor),
    ExposingValue(Cursor),
    ExposingOperator(Cursor),
    ExposingOperatorReserved(BadOperator, Cursor),
    ExposingOperatorRightParen(Cursor),
    ExposingTypePrivacy(Cursor),
    ExposingEnd(Cursor),
    ExposingIndentEnd(Cursor),
    ExposingIndentValue(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decl {
    DeclStart(Cursor),
    DeclSpace(Space, Cursor),
    Port(Port, Cursor),
    DeclType(DeclType, Cursor),
    DeclDef(Name, DeclDef, Cursor),
    DeclFreshLineAfterDocComment(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclDef {
    DeclDefSpace(Space, Cursor),
    DeclDefEquals(Cursor),
    DeclDefType(Type, Cursor),
    DeclDefArg(Pattern, Cursor),
    DeclDefBody(Expr, Cursor),
    DeclDefNameRepeat(Cursor),
    DeclDefNameMatch(Name, Cursor),
    DeclDefIndentType(Cursor),
    DeclDefIndentEquals(Cursor),
    DeclDefIndentBody(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Port {
    PortSpace(Space, Cursor),
    PortName(Cursor),
    PortColon(Cursor),
    PortType(Type, Cursor),
    PortIndentName(Cursor),
    PortIndentColon(Cursor),
    PortIndentType(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclType {
    DtSpace(Space, Cursor),
    DtName(Cursor),
    DtAlias(TypeAlias, Cursor),
    DtUnion(CustomType, Cursor),
    DtIndentName(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeAlias {
    AliasSpace(Space, Cursor),
    AliasName(Cursor),
    AliasEquals(Cursor),
    AliasBody(Type, Cursor),
    AliasIndentEquals(Cursor),
    AliasIndentBody(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CustomType {
    CtSpace(Space, Cursor),
    CtName(Cursor),
    CtEquals(Cursor),
    CtBar(Cursor),
    CtVariant(Cursor),
    CtVariantArg(Type, Cursor),
    CtIndentEquals(Cursor),
    CtIndentBar(Cursor),
    CtIndentAfterBar(Cursor),
    CtIndentAfterEquals(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Let(Box<Let>, Cursor),
    Case(Box<Case>, Cursor),
    If(Box<If>, Cursor),
    List(Box<List>, Cursor),
    Record(Box<Record>, Cursor),
    Tuple(Box<Tuple>, Cursor),
    Func(Box<Func>, Cursor),
    Dot(Cursor),
    Access(Cursor),
    OperatorRight(Name, Cursor),
    OperatorReserved(BadOperator, Cursor),
    Start(Cursor),
    Char(Char, Cursor),
    String(String, Cursor),
    Number(Number, Cursor),
    Space(Space, Cursor),
    /// 0.19.1's `EndlessShader`.
    ShaderEndless(Cursor),
    ShaderNotUtf8(Cursor),
    /// Parsec's message, which nomi's GLSL parser doesn't give yet.
    ShaderProblem(Name, Cursor),
    IndentOperatorRight(Name, Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    RecordOpen(Cursor),
    RecordEnd(Cursor),
    RecordField(Cursor),
    RecordEquals(Cursor),
    RecordExpr(Expr, Cursor),
    RecordSpace(Space, Cursor),
    RecordIndentOpen(Cursor),
    RecordIndentEnd(Cursor),
    RecordIndentField(Cursor),
    RecordIndentEquals(Cursor),
    RecordIndentExpr(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tuple {
    TupleExpr(Expr, Cursor),
    TupleSpace(Space, Cursor),
    TupleEnd(Cursor),
    TupleOperatorClose(Cursor),
    TupleOperatorReserved(BadOperator, Cursor),
    TupleIndentExpr1(Cursor),
    TupleIndentExprN(Cursor),
    TupleIndentEnd(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum List {
    ListSpace(Space, Cursor),
    ListOpen(Cursor),
    ListExpr(Expr, Cursor),
    ListEnd(Cursor),
    ListIndentOpen(Cursor),
    ListIndentEnd(Cursor),
    ListIndentExpr(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Func {
    FuncSpace(Space, Cursor),
    FuncArg(Pattern, Cursor),
    FuncBody(Expr, Cursor),
    FuncArrow(Cursor),
    FuncIndentArg(Cursor),
    FuncIndentArrow(Cursor),
    FuncIndentBody(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Case {
    CaseSpace(Space, Cursor),
    CaseOf(Cursor),
    CasePattern(Pattern, Cursor),
    CaseArrow(Cursor),
    CaseExpr(Expr, Cursor),
    CaseBranch(Expr, Cursor),
    CaseIndentOf(Cursor),
    CaseIndentExpr(Cursor),
    CaseIndentPattern(Cursor),
    CaseIndentArrow(Cursor),
    CaseIndentBranch(Cursor),
    CasePatternAlignment(Indent, Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum If {
    IfSpace(Space, Cursor),
    IfThen(Cursor),
    IfElse(Cursor),
    IfElseBranchStart(Cursor),
    IfCondition(Expr, Cursor),
    IfThenBranch(Expr, Cursor),
    IfElseBranch(Expr, Cursor),
    IfIndentCondition(Cursor),
    IfIndentThen(Cursor),
    IfIndentThenBranch(Cursor),
    IfIndentElseBranch(Cursor),
    IfIndentElse(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Let {
    LetSpace(Space, Cursor),
    LetIn(Cursor),
    LetDefAlignment(Indent, Cursor),
    LetDefName(Cursor),
    LetDef(Name, Def, Cursor),
    LetDestruct(Destruct, Cursor),
    LetBody(Expr, Cursor),
    LetIndentDef(Cursor),
    LetIndentIn(Cursor),
    LetIndentBody(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Def {
    DefSpace(Space, Cursor),
    DefType(Type, Cursor),
    DefNameRepeat(Cursor),
    DefNameMatch(Name, Cursor),
    DefArg(Pattern, Cursor),
    DefEquals(Cursor),
    DefBody(Expr, Cursor),
    DefIndentEquals(Cursor),
    DefIndentType(Cursor),
    DefIndentBody(Cursor),
    DefAlignment(Indent, Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destruct {
    DestructSpace(Space, Cursor),
    DestructPattern(Pattern, Cursor),
    DestructEquals(Cursor),
    DestructBody(Expr, Cursor),
    DestructIndentEquals(Cursor),
    DestructIndentBody(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pattern {
    PRecord(PRecord, Cursor),
    PTuple(Box<PTuple>, Cursor),
    PList(Box<PList>, Cursor),
    PStart(Cursor),
    PChar(Char, Cursor),
    PString(String, Cursor),
    PNumber(Number, Cursor),
    /// The float's width.
    PFloat(u32, Cursor),
    PAlias(Cursor),
    /// The name, and the width elm reports, which differs by release.
    PWildcardNotVar(Name, u32, Cursor),
    PSpace(Space, Cursor),
    PIndentStart(Cursor),
    PIndentAlias(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PRecord {
    PRecordOpen(Cursor),
    PRecordEnd(Cursor),
    PRecordField(Cursor),
    PRecordSpace(Space, Cursor),
    PRecordIndentOpen(Cursor),
    PRecordIndentEnd(Cursor),
    PRecordIndentField(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PTuple {
    PTupleOpen(Cursor),
    PTupleEnd(Cursor),
    PTupleExpr(Pattern, Cursor),
    PTupleSpace(Space, Cursor),
    PTupleIndentEnd(Cursor),
    PTupleIndentExpr1(Cursor),
    PTupleIndentExprN(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PList {
    PListOpen(Cursor),
    PListEnd(Cursor),
    PListExpr(Pattern, Cursor),
    PListSpace(Space, Cursor),
    PListIndentOpen(Cursor),
    PListIndentEnd(Cursor),
    PListIndentExpr(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    TRecord(Box<TRecord>, Cursor),
    TTuple(Box<TTuple>, Cursor),
    TStart(Cursor),
    TSpace(Space, Cursor),
    TIndentStart(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TRecord {
    TRecordOpen(Cursor),
    TRecordEnd(Cursor),
    TRecordField(Cursor),
    TRecordColon(Cursor),
    TRecordType(Type, Cursor),
    TRecordSpace(Space, Cursor),
    TRecordIndentOpen(Cursor),
    TRecordIndentField(Cursor),
    TRecordIndentColon(Cursor),
    TRecordIndentType(Cursor),
    TRecordIndentEnd(Cursor),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TTuple {
    TTupleOpen(Cursor),
    TTupleEnd(Cursor),
    TTupleType(Type, Cursor),
    TTupleSpace(Space, Cursor),
    TTupleIndentType1(Cursor),
    TTupleIndentTypeN(Cursor),
    TTupleIndentEnd(Cursor),
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
    /// The width, the number of digits, and the code they spell.
    BadUnicodeLength(u32, u32, u32),
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
