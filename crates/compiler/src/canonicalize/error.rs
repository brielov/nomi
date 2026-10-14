//! What canonicalization finds wrong (`Reporting/Error/Canonicalize.hs`'s
//! `Error`). One type serves every release: where 0.19.3 splits a
//! constructor by the kind of name, a [`NameKind`] says which, and lists
//! of homes and names keep the order canonicalization met them in, which
//! the report sorts as each release does.

use nomi_base::TextRange;

use crate::canonical::Home;
use crate::hir::{Spanned, TypeId};

/// Which sort of name: 0.19.3 reports each sort its own way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameKind {
    Value,
    Type,
    /// A union's constructor.
    Variant,
    Operator,
}

/// What has the wrong number of arguments: a type, or a constructor in a
/// pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArityContext {
    Type,
    Pattern,
}

/// What a name written twice names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Duplicated {
    /// A top-level value or port.
    Decl,
    Type,
    Ctor,
    /// A record's field.
    Field,
}

impl Duplicated {
    /// The error of a name written at `first` and again at `second`, as
    /// `Dict::detect` takes it.
    pub(super) fn error(self) -> impl Fn(&str, TextRange, TextRange) -> Error {
        move |name, first, second| Error::Duplicate {
            what: self,
            name: name.to_owned(),
            first,
            second,
        }
    }
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

/// `Error`: one way a module's names can be wrong.
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
    /// `DuplicateDecl`, `DuplicateType`, `DuplicateCtor` and
    /// `DuplicateField`, which differ in their report's wording alone.
    Duplicate {
        what: Duplicated,
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
        body: TypeId,
        others: Vec<String>,
    },
    RecursiveDecl {
        range: TextRange,
        name: String,
        others: Vec<String>,
    },
    RecursiveLet {
        name: Spanned,
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
        first: Spanned,
        rest: Vec<Spanned>,
    },
    TypeVarsMessedUpInAlias {
        range: TextRange,
        name: String,
        vars: Vec<String>,
        unused: Vec<Spanned>,
        unbound: Vec<Spanned>,
    },
}

/// A canonicalization result: upstream's `Result` with its errors in
/// order. `?` is its monadic bind, which stops at the first error;
/// [`both`] and [`all`] are its applicative combinations, which keep
/// going and collect every error, left to right.
pub type R<T> = Result<T, Vec<Error>>;

/// `Result.throw`: a result that is only `error`.
pub(super) fn throw<T>(error: Error) -> R<T> {
    Err(vec![error])
}

/// `value`, unless making it found `errors`.
pub(super) fn collected<T>(value: T, errors: Vec<Error>) -> R<T> {
    if errors.is_empty() {
        Ok(value)
    } else {
        Err(errors)
    }
}

/// `f <$> a <*> b`: both results, or the errors of either, `a`'s first.
pub(super) fn both<A, B>(a: R<A>, b: R<B>) -> R<(A, B)> {
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
pub(super) fn all<T>(results: impl IntoIterator<Item = R<T>>) -> R<Vec<T>> {
    let mut values = Vec::new();
    let mut errors = Vec::new();
    for result in results {
        match result {
            Ok(value) => values.push(value),
            Err(e) => errors.extend(e),
        }
    }
    collected(values, errors)
}
