//! What canonicalization finds wrong (`Reporting/Error/Canonicalize.hs`'s
//! `Error`). One type serves every release: where 0.19.3 splits a
//! constructor by the kind of name, a [`NameKind`] says which, and lists
//! of homes and names keep the order canonicalization met them in, which
//! the report sorts as each release does.

use nomi_base::TextRange;

use crate::canonical::{Home, Type};

/// A name and where it is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct At {
    pub name: String,
    pub range: TextRange,
}

/// Which sort of name: 0.19.3 reports each sort its own way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameKind {
    Value,
    Type,
    /// A union's constructor.
    Variant,
    Operator,
    /// A name in a pattern, as an export error reports one.
    Pattern,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArityContext {
    Type,
    Pattern,
}

/// Where a pattern binds a name twice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DuplicatePatternContext {
    LambdaArgs,
    FuncArgs(String),
    CaseBranch,
    LetBinding,
    Destruct,
}

/// Why a port can't carry a type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvalidPayload {
    ExtendedRecord,
    Function,
    TypeVariable(String),
    UnsupportedType(String),
}

/// Why a port's type isn't a port's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortProblem {
    CmdNoArg,
    CmdExtraArgs(usize),
    CmdBadMsg,
    SubBad,
    NotCmdOrSub,
}

/// The names a missing one may be a typo of: those in scope unqualified,
/// and those under each qualifier.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PossibleNames {
    pub locals: Vec<String>,
    pub qualified: Vec<(String, Vec<String>)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    AnnotationTooShort {
        range: TextRange,
        name: String,
        index: usize,
        leftover: usize,
    },
    /// A value, type or constructor that more than one import exposes.
    Ambiguous {
        kind: NameKind,
        range: TextRange,
        prefix: Option<String>,
        name: String,
        homes: Vec<Home>,
    },
    AmbiguousBinop {
        range: TextRange,
        op: String,
        homes: Vec<Home>,
    },
    BadArity {
        range: TextRange,
        context: ArityContext,
        name: String,
        expected: usize,
        actual: usize,
    },
    Binop {
        range: TextRange,
        left: String,
        right: String,
    },
    DuplicateDecl {
        name: String,
        first: TextRange,
        second: TextRange,
    },
    DuplicateType {
        name: String,
        first: TextRange,
        second: TextRange,
    },
    DuplicateCtor {
        name: String,
        first: TextRange,
        second: TextRange,
    },
    DuplicateBinop {
        name: String,
        first: TextRange,
        second: TextRange,
    },
    DuplicateField {
        name: String,
        first: TextRange,
        second: TextRange,
    },
    DuplicateAliasArg {
        alias: String,
        name: String,
        first: TextRange,
        second: TextRange,
    },
    DuplicateUnionArg {
        union: String,
        name: String,
        first: TextRange,
        second: TextRange,
    },
    DuplicatePattern {
        context: DuplicatePatternContext,
        name: String,
        first: TextRange,
        second: TextRange,
    },
    EffectNotFound {
        range: TextRange,
        name: String,
    },
    EffectFunctionNotFound {
        range: TextRange,
        name: String,
    },
    ExportDuplicate {
        kind: NameKind,
        name: String,
        first: TextRange,
        second: TextRange,
    },
    ExportNotFound {
        range: TextRange,
        kind: NameKind,
        name: String,
        possible: Vec<String>,
    },
    ExportOpenAlias {
        range: TextRange,
        name: String,
    },
    ImportCtorByName {
        range: TextRange,
        ctor: String,
        union: String,
    },
    ImportOpenAlias {
        range: TextRange,
        name: String,
    },
    ImportExposingNotFound {
        range: TextRange,
        kind: NameKind,
        home: Home,
        name: String,
        possible: Vec<String>,
    },
    NotFound {
        kind: NameKind,
        range: TextRange,
        prefix: Option<String>,
        name: String,
        possible: PossibleNames,
    },
    NotFoundBinop {
        range: TextRange,
        op: String,
        possible: Vec<String>,
    },
    PatternHasRecordCtor {
        range: TextRange,
        name: String,
    },
    PortPayloadInvalid {
        range: TextRange,
        name: String,
        bad: Type,
        problem: InvalidPayload,
    },
    PortTypeInvalid {
        range: TextRange,
        name: String,
        problem: PortProblem,
    },
    RecursiveAlias {
        range: TextRange,
        name: String,
        args: Vec<String>,
        /// The alias's body, as written.
        body: crate::hir::TypeId,
        others: Vec<String>,
    },
    RecursiveDecl {
        range: TextRange,
        name: String,
        others: Vec<String>,
    },
    RecursiveLet {
        name: At,
        others: Vec<String>,
    },
    Shadowing {
        name: String,
        first: TextRange,
        second: TextRange,
    },
    TupleLargerThanThree {
        range: TextRange,
    },
    TypeVarsUnboundInUnion {
        range: TextRange,
        name: String,
        vars: Vec<String>,
        first: At,
        rest: Vec<At>,
    },
    TypeVarsMessedUpInAlias {
        range: TextRange,
        name: String,
        vars: Vec<String>,
        unused: Vec<At>,
        unbound: Vec<At>,
    },
}

/// The errors of a pass, in the order upstream's `Result` keeps them.
pub type Errors = Vec<Error>;

/// A canonicalization result: upstream's `Result` with its errors in
/// order. `?` is its monadic bind, which stops at the first error;
/// [`both`] and [`all`] are its applicative combinations, which keep
/// going and collect every error, left to right.
pub type R<T> = Result<T, Errors>;

pub fn throw<T>(error: Error) -> R<T> {
    Err(vec![error])
}

/// `f <$> a <*> b`: both results, or the errors of either, `a`'s first.
pub fn both<A, B>(a: R<A>, b: R<B>) -> R<(A, B)> {
    match (a, b) {
        (Ok(a), Ok(b)) => Ok((a, b)),
        (Err(e), Ok(_)) | (Ok(_), Err(e)) => Err(e),
        (Err(mut e1), Err(e2)) => {
            e1.extend(e2);
            Err(e1)
        }
    }
}

/// `traverse`: every result, or every error, in order.
pub fn all<T>(results: impl IntoIterator<Item = R<T>>) -> R<Vec<T>> {
    let mut values = Vec::new();
    let mut errors = Vec::new();
    for result in results {
        match result {
            Ok(value) => values.push(value),
            Err(e) => errors.extend(e),
        }
    }
    if errors.is_empty() {
        Ok(values)
    } else {
        Err(errors)
    }
}
