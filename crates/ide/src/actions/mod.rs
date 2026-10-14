//! Code actions as elm-language-server 2.10.0 offers them
//! (`codeActionProvider.ts`): for each diagnostic the editor sends, the
//! fixes registered for its code, in registration order
//! (`codeAction/index.ts`), each followed by a fix-all when the program
//! has another diagnostic of a code it fixes; then the refactors at the
//! range; then elm make's own suggestions.
//!
//! A fix-all lists the program's diagnostics as elm-language-server's
//! own type checker has them, not elm's: nomi writes them from elm's
//! reports with that checker's messages (`diagnostics.ts`).

use std::cell::{OnceCell, RefCell};
use std::path::Path;

use nomi_base::{TextRange, TextSize};
use nomi_compiler::types::ExprTypes;
use nomi_engine::Snapshot;
use nomi_engine::index::{Index, PackageModules};
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

use crate::completion::Context;
use crate::problems::{Code, Problem, Severity, Source};
use crate::{Edit, FileEdits};

mod extract;
mod import;
mod missing;
mod moving;
mod refactor;
mod unused;
mod usage;

use extract::{AliasOf, Place};
use import::{import_actions, import_all, install_actions};
use missing::{
    case_branch_actions, case_branch_all, is_missing_field, is_missing_patterns, missing_field,
    missing_patterns, record_field_actions, record_field_all,
};
use refactor::{annotation_actions, annotation_all, refactor_edits_in, refactors};
use unused::{RemovedImports, parameter_removal_actions, unused_actions, unused_all};
use usage::{
    external_function_actions, external_function_all, local_function_actions, local_function_all,
    parameter_actions, union_actions, union_all,
};

pub use moving::{Destination, MoveEdit, move_destinations, move_function};
pub use refactor::{Exposure, exposing_edit, refactor_edits};

/// A diagnostic as code actions see it: one the editor sends, or one of
/// the program's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnosed {
    pub code: DiagnosticCode,
    pub message: String,
    pub range: TextRange,
    pub severity: Severity,
    /// Who reports it, where it's one elm-language-server knows.
    pub source: Option<Source>,
    /// Whether it marks code nothing needs.
    pub unnecessary: bool,
    /// The edits that fix it, which elm-review gives.
    pub fixes: Vec<Edit>,
}

/// A diagnostic's code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticCode {
    /// One of elm-language-server's.
    Known(Code),
    /// Another tool's, which only elm's reports' fixes look past.
    Other,
    /// None, which leaves the diagnostic out of the fixes and refactors.
    Missing,
}

impl Diagnosed {
    fn of(problem: &Problem) -> Diagnosed {
        Diagnosed {
            code: DiagnosticCode::Known(problem.code),
            message: problem.message.clone(),
            range: problem.range,
            severity: problem.severity,
            source: Some(problem.code.source()),
            unnecessary: problem.unnecessary,
            fixes: problem.fixes.clone(),
        }
    }

    /// Whether its code is `code`.
    fn is(&self, code: Code) -> bool {
        self.code == DiagnosticCode::Known(code)
    }
}

/// What a code action request asks about.
#[derive(Debug)]
pub struct Request<'a> {
    pub range: TextRange,
    /// The diagnostics the editor sends along.
    pub given: &'a [Diagnosed],
    /// The file's latest problems, as published.
    pub problems: &'a [Problem],
    pub packages: &'a PackageModules,
    pub resolve: Resolve,
    pub moving: Moving,
}

/// When a refactor's edits are made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolve {
    /// With its action.
    Now,
    /// When the editor resolves its action.
    Later,
}

/// Whether the editor moves functions, asking where to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moving {
    Supported,
    Unsupported,
}

/// An action's kind, as the protocol names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    QuickFix,
    Refactor,
    RefactorExtract,
    RefactorMove,
}

/// The diagnostics an action names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionDiagnostics {
    /// The one the editor sent, by its index.
    Given(usize),
    /// The program's, for a fix-all.
    Program(Vec<Diagnosed>),
}

/// What an action carries for elm-language-server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionData {
    /// A fix, and whether it fixes every diagnostic.
    Fix { id: FixId, all: bool },
    /// A refactor, at the range it was asked about, which resolving it
    /// reads again.
    Refactor {
        action: RefactorAction,
        range: TextRange,
        /// Where the editor starts renaming what it made.
        rename: Option<RenameAt>,
    },
    /// Elm make's suggestions carry nothing.
    None,
}

/// A fix, as elm-language-server registers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixId {
    Import,
    MakeDeclarationFromUsage,
    MakeExternalDeclarationFromUsage,
    AddTypeAnnotation,
    AddMissingRecordField,
    AddMissingCaseBranches,
    ElmReviewFix,
    AddMissingUnionType,
    AddNewFunctionParameter,
    RemoveUnused,
    RemoveUnusedParameter,
    InstallPackage,
}

impl FixId {
    /// Its id, as elm-language-server writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            FixId::Import => "import",
            FixId::MakeDeclarationFromUsage => "make_declaration_from_usage",
            FixId::MakeExternalDeclarationFromUsage => "make_external_declaration_from_usage",
            FixId::AddTypeAnnotation => "add_type_annotation",
            FixId::AddMissingRecordField => "add_missing_record_field",
            FixId::AddMissingCaseBranches => "add_missing_case_branches",
            FixId::ElmReviewFix => "elm_review_fix",
            FixId::AddMissingUnionType => "add_missing_union_type",
            FixId::AddNewFunctionParameter => "add_new_function_parameter",
            FixId::RemoveUnused => "remove_unused",
            FixId::RemoveUnusedParameter => "remove_unused_parameter",
            FixId::InstallPackage => "install_package",
        }
    }
}

/// A refactor's action, which names the refactor it's one of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefactorAction {
    AddTypeAnnotation,
    ExtractFunction(Place),
    ExtractTypeAlias(AliasOf),
    ExposeFunction,
    UnexposeFunction,
    ExposeType,
    UnexposeType,
    ExposeTypeWithVariants,
    MoveFunction,
}

impl RefactorAction {
    const ALL: [RefactorAction; 12] = [
        RefactorAction::AddTypeAnnotation,
        RefactorAction::ExtractFunction(Place::Let),
        RefactorAction::ExtractFunction(Place::TopLevel),
        RefactorAction::ExtractTypeAlias(AliasOf::Type),
        RefactorAction::ExtractTypeAlias(AliasOf::Parenthesized),
        RefactorAction::ExtractTypeAlias(AliasOf::Parts),
        RefactorAction::ExposeFunction,
        RefactorAction::UnexposeFunction,
        RefactorAction::ExposeType,
        RefactorAction::UnexposeType,
        RefactorAction::ExposeTypeWithVariants,
        RefactorAction::MoveFunction,
    ];

    /// The action as elm-language-server names it.
    pub fn as_str(self) -> &'static str {
        match self {
            RefactorAction::AddTypeAnnotation => "add_type_annotation",
            RefactorAction::ExtractFunction(Place::Let) => "extract_let",
            RefactorAction::ExtractFunction(Place::TopLevel) => "extract_top_level",
            RefactorAction::ExtractTypeAlias(AliasOf::Type) => "extract_type_alias",
            RefactorAction::ExtractTypeAlias(AliasOf::Parenthesized) => {
                "extract_type_alias_parenthesized_expr"
            }
            RefactorAction::ExtractTypeAlias(AliasOf::Parts) => {
                "extract_type_alias_partial_type_expr"
            }
            RefactorAction::ExposeFunction => "expose_function",
            RefactorAction::UnexposeFunction => "unexpose_function",
            RefactorAction::ExposeType => "expose_type",
            RefactorAction::UnexposeType => "unexpose_type",
            RefactorAction::ExposeTypeWithVariants => "expose_type_with_variants",
            RefactorAction::MoveFunction => "move_function",
        }
    }

    /// The action `name` names, where it's one.
    pub fn parse(name: &str) -> Option<RefactorAction> {
        RefactorAction::ALL
            .into_iter()
            .find(|action| action.as_str() == name)
    }

    /// The refactor it's one of, as elm-language-server names it.
    pub fn refactor(self) -> &'static str {
        match self {
            RefactorAction::AddTypeAnnotation => "add_type_annotation",
            RefactorAction::ExtractFunction(_) => "extract_function",
            RefactorAction::ExtractTypeAlias(_) => "extract_type_alias",
            RefactorAction::ExposeFunction
            | RefactorAction::UnexposeFunction
            | RefactorAction::ExposeType
            | RefactorAction::UnexposeType
            | RefactorAction::ExposeTypeWithVariants => "expose_unexpose",
            RefactorAction::MoveFunction => "move_function",
        }
    }

    fn kind(self) -> ActionKind {
        match self {
            RefactorAction::AddTypeAnnotation
            | RefactorAction::ExtractFunction(_)
            | RefactorAction::ExtractTypeAlias(_) => ActionKind::RefactorExtract,
            RefactorAction::ExposeFunction
            | RefactorAction::UnexposeFunction
            | RefactorAction::ExposeType
            | RefactorAction::UnexposeType
            | RefactorAction::ExposeTypeWithVariants => ActionKind::Refactor,
            RefactorAction::MoveFunction => ActionKind::RefactorMove,
        }
    }
}

/// A command the editor runs for an action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionCommand {
    /// Add `package` to the project.
    InstallPackage { package: String },
    /// Move the function `name`, asking the editor where to.
    MoveFunction { name: String },
}

/// A position after a refactor: `lines` below and `columns` right of
/// where `from` is now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenameAt {
    pub from: TextSize,
    pub lines: u32,
    pub columns: u32,
}

/// A refactor at a range: `IRefactorCodeAction` before its edits.
struct Refactor {
    action: RefactorAction,
    title: String,
    command: Option<ActionCommand>,
}

impl Refactor {
    fn new(action: RefactorAction, title: impl Into<String>) -> Refactor {
        Refactor {
            action,
            title: title.into(),
            command: None,
        }
    }
}

/// A code action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub title: String,
    pub kind: ActionKind,
    pub diagnostics: Option<ActionDiagnostics>,
    /// Each file's edits; none for an action whose edit comes later.
    pub edits: Option<FileEdits>,
    pub data: ActionData,
    /// Whether the editor should prefer it; none for elm make's.
    pub preferred: Option<bool>,
    pub command: Option<ActionCommand>,
}

/// A fix's registration: `ICodeActionRegistration`.
struct Fix {
    id: FixId,
    codes: &'static [Code],
    /// elm's reports it answers too, where elm-language-server answers
    /// its own checker's (DECISIONS 2026-10-12).
    reports: Option<fn(&Diagnosed) -> bool>,
    actions: fn(&Fixing<'_>, TextRange) -> Vec<Offer>,
    all: Option<(&'static str, AllEdits)>,
}

impl Fix {
    /// Whether it fixes `diagnostic`: by its code, or as elm's report.
    fn applies(&self, diagnostic: &Diagnosed) -> bool {
        self.fixes(diagnostic)
            || (self.reports)
                .is_some_and(|is| diagnostic.source == Some(Source::Elm) && is(diagnostic))
    }

    /// Whether it's registered for `diagnostic`'s code.
    fn fixes(&self, diagnostic: &Diagnosed) -> bool {
        self.codes.iter().any(|code| diagnostic.is(*code))
    }
}

/// A fix's action.
struct Offer {
    title: String,
    edits: FileEdits,
    command: Option<ActionCommand>,
}

impl Offer {
    fn new(title: impl Into<String>, edits: FileEdits) -> Offer {
        Offer {
            title: title.into(),
            edits,
            command: None,
        }
    }
}

/// A fix-all's edits for one diagnostic, added to those so far: in the
/// file, sorted at the end, or in any files, each file's in order.
#[derive(Clone, Copy)]
enum AllEdits {
    Here(fn(&Fixing<'_>, &Diagnosed, &mut Vec<Edit>)),
    Files(fn(&Fixing<'_>, &Diagnosed, &mut FileEdits)),
}

/// How strongly the editor should prefer an action: `isPreferredFix`'s
/// map, keyed by a fix's id or a refactor's name.
#[derive(Clone, Copy)]
struct Preferred {
    priority: u8,
    /// Whether only one action of its key may be preferred.
    only_one: bool,
}

impl Preferred {
    /// The preference of an action carrying `data`, where it has one.
    /// Each priority is one key's.
    fn of(data: &ActionData) -> Option<Preferred> {
        use RefactorAction as R;
        let (priority, only_one) = match data {
            ActionData::Fix {
                id: FixId::Import,
                all: false,
            } => (1, true),
            ActionData::Refactor {
                action:
                    R::ExposeFunction
                    | R::UnexposeFunction
                    | R::ExposeType
                    | R::UnexposeType
                    | R::ExposeTypeWithVariants,
                ..
            } => (2, false),
            ActionData::Fix {
                id: FixId::AddTypeAnnotation,
                all: false,
            }
            | ActionData::Refactor {
                action: R::AddTypeAnnotation,
                ..
            } => (3, false),
            ActionData::Fix { .. } | ActionData::Refactor { .. } | ActionData::None => {
                return None;
            }
        };
        Some(Preferred { priority, only_one })
    }
}

/// The fixes, in elm-language-server's registration order.
const FIXES: &[Fix] = &[
    Fix {
        id: FixId::Import,
        codes: &[Code::MissingValue],
        reports: None,
        actions: import_actions,
        all: Some(("Add all missing imports", AllEdits::Here(import_all))),
    },
    Fix {
        id: FixId::MakeDeclarationFromUsage,
        codes: &[Code::MissingValue],
        reports: None,
        actions: local_function_actions,
        all: Some((
            "Create all missing local functions",
            AllEdits::Here(local_function_all),
        )),
    },
    Fix {
        id: FixId::MakeExternalDeclarationFromUsage,
        codes: &[Code::MissingValue],
        reports: None,
        actions: external_function_actions,
        all: Some((
            "Create all missing external functions",
            AllEdits::Files(external_function_all),
        )),
    },
    Fix {
        id: FixId::AddTypeAnnotation,
        codes: &[Code::MissingTypeAnnotation],
        reports: None,
        actions: annotation_actions,
        all: Some((
            "Add all missing type annotations",
            AllEdits::Here(annotation_all),
        )),
    },
    Fix {
        id: FixId::AddMissingRecordField,
        codes: &[Code::RecordField],
        reports: Some(is_missing_field),
        actions: record_field_actions,
        all: Some((
            "Create all missing record fields",
            AllEdits::Files(record_field_all),
        )),
    },
    Fix {
        id: FixId::AddMissingCaseBranches,
        codes: &[Code::IncompleteCasePattern],
        reports: Some(is_missing_patterns),
        actions: case_branch_actions,
        all: Some((
            "Add all missing case branches",
            AllEdits::Here(case_branch_all),
        )),
    },
    Fix {
        id: FixId::ElmReviewFix,
        codes: &[Code::ElmReview],
        reports: None,
        actions: review_actions,
        all: None,
    },
    Fix {
        id: FixId::AddMissingUnionType,
        codes: &[Code::MissingValue],
        reports: None,
        actions: union_actions,
        all: Some((
            "Create all missing union constructors",
            AllEdits::Files(union_all),
        )),
    },
    Fix {
        id: FixId::AddNewFunctionParameter,
        codes: &[Code::MissingValue],
        reports: None,
        actions: parameter_actions,
        all: None,
    },
    Fix {
        id: FixId::RemoveUnused,
        codes: &[
            Code::UnusedImport,
            Code::UnusedImportedValue,
            Code::UnusedAlias,
            Code::UnusedTopLevel,
            Code::UnusedPattern,
            Code::UnusedValueConstructor,
            Code::UnusedTypeAlias,
        ],
        reports: None,
        actions: unused_actions,
        all: Some((
            "Remove all reported unused code",
            AllEdits::Here(unused_all),
        )),
    },
    Fix {
        id: FixId::RemoveUnusedParameter,
        codes: &[Code::UnusedPattern],
        reports: None,
        actions: parameter_removal_actions,
        all: None,
    },
    Fix {
        id: FixId::InstallPackage,
        codes: &[Code::ImportResolve],
        reports: None,
        actions: install_actions,
        all: None,
    },
];

/// The module with its missing values made `Debug.todo`: its types, and
/// the ranges the missing values had, in order.
struct Usage {
    types: ExprTypes,
    missing: Vec<TextRange>,
}

/// What fixes read: the file, the request, and its program diagnostics.
struct Fixing<'a> {
    context: Context<'a>,
    request: &'a Request<'a>,
    program: Vec<Diagnosed>,
    /// What a fix-all removes from each import so far.
    removed_imports: RefCell<RemovedImports>,
    /// The types usage gives the missing values, inferred once.
    usage: OnceCell<Option<Usage>>,
}

/// The code actions for `request` in the file at `path`.
pub fn code_actions(
    snapshot: &Snapshot,
    index: &Index,
    path: &Path,
    request: &Request<'_>,
) -> Vec<Action> {
    let Some(module) = index.module(path) else {
        return Vec::new();
    };
    // None when the module doesn't parse at the top.
    if module
        .root()
        .first_child()
        .is_some_and(|n| n.kind() == SyntaxKind::Error)
    {
        return Vec::new();
    }
    let fixing = Fixing {
        context: Context::new(snapshot, index, module, path),
        request,
        program: program_diagnostics(&module.text, &module.root(), request.problems),
        removed_imports: RefCell::default(),
        usage: OnceCell::new(),
    };
    let (mut results, range) = fix_actions(&fixing);
    results.extend(refactor_actions(&fixing, range));
    let preferred: Vec<bool> = (0..results.len())
        .map(|i| is_preferred(&results, i))
        .collect();
    for (action, preferred) in results.iter_mut().zip(preferred) {
        action.preferred = Some(preferred);
    }
    for (i, given) in request.given.iter().enumerate() {
        results.extend(elm_make_actions(path, i, given));
    }
    results
}

/// The fixes of each diagnostic the editor sent, and the range the
/// refactors are asked about: each diagnostic's becomes the request's, so
/// they see the last one's.
fn fix_actions(fixing: &Fixing<'_>) -> (Vec<Action>, TextRange) {
    let mut results = Vec::new();
    let mut range = fixing.request.range;
    for (i, given) in fixing.request.given.iter().enumerate() {
        if given.code == DiagnosticCode::Missing {
            continue;
        }
        range = given.range;
        for fix in FIXES.iter().filter(|f| f.applies(given)) {
            let found = (fix.actions)(fixing, given.range);
            if found.is_empty() {
                continue;
            }
            results.extend(found.into_iter().map(|offer| Action {
                title: offer.title,
                kind: ActionKind::QuickFix,
                diagnostics: Some(ActionDiagnostics::Given(i)),
                edits: Some(offer.edits),
                data: ActionData::Fix {
                    id: fix.id,
                    all: false,
                },
                preferred: None,
                command: offer.command,
            }));
            // Another diagnostic it fixes. elm's report and the checker's
            // form of it nomi writes are one.
            let same = |d: &Diagnosed| {
                d == given || (d.range == given.range && fix.reports.is_some_and(|is| is(given)))
            };
            let others = (fixing.program.iter()).any(|d| !same(d) && fix.fixes(d));
            if let Some((title, edits_for)) = fix.all
                && others
            {
                results.push(fix_all(fixing, fix, title, edits_for));
            }
        }
    }
    (results, range)
}

/// The refactors at `range`, their edits made unless the editor resolves
/// them later.
fn refactor_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Action> {
    let path = fixing.context.path;
    (refactors(fixing, range).into_iter())
        .map(|refactor| {
            let edits = match fixing.request.resolve {
                Resolve::Now => refactor_edits_in(&fixing.context, refactor.action, range),
                Resolve::Later => None,
            };
            let rename = edits.as_ref().and_then(|(_, rename)| *rename);
            Action {
                title: refactor.title,
                kind: refactor.action.kind(),
                diagnostics: None,
                edits: edits.map(|(edits, _)| FileEdits::of(path, edits)),
                data: ActionData::Refactor {
                    action: refactor.action,
                    range,
                    rename,
                },
                preferred: None,
                command: refactor.command,
            }
        })
        .collect()
}

/// `getDiagnostics`: the checker's, its suggestions, then the lint rules'.
fn program_diagnostics(text: &str, root: &SyntaxNode, problems: &[Problem]) -> Vec<Diagnosed> {
    let mut found = Vec::new();
    for problem in problems.iter().filter(|p| p.code.source() == Source::Elm) {
        let elm = Diagnosed::of(problem);
        if problem.code == Code::MissingValue {
            let name = &text[problem.range];
            found.push(Diagnosed {
                message: format!("No definition found for `{name}`."),
                ..elm
            });
        } else if is_missing_patterns(&elm) {
            let missing: String = (missing_patterns(&elm.message).iter())
                .map(|p| format!("  {p}\n"))
                .collect();
            found.push(Diagnosed {
                code: DiagnosticCode::Known(Code::IncompleteCasePattern),
                message: format!(
                    "This `case` does not have branches for all possibilities.\nMissing possibilities include:\n{missing}"
                ),
                ..elm
            });
        } else if is_missing_field(&elm)
            && let Some((_, field)) = missing_field(root, &elm)
        {
            found.push(Diagnosed {
                code: DiagnosticCode::Known(Code::RecordField),
                message: format!("The record does not have a `{}` field.", field.text()),
                range: field.text_range(),
                ..elm
            });
        }
    }
    found.extend(
        (problems.iter())
            .filter(|p| p.code == Code::MissingTypeAnnotation)
            .map(Diagnosed::of),
    );
    found.extend(
        (problems.iter())
            .filter(|p| p.code.source() == Source::ElmLs)
            .map(Diagnosed::of),
    );
    found
}

/// `getFixAllCodeAction`: the edits for each program diagnostic of the
/// fix's codes, sorted, each starting no earlier than the last ends.
fn fix_all(fixing: &Fixing<'_>, fix: &Fix, title: &str, edits_for: AllEdits) -> Action {
    let mut diagnostics = Vec::new();
    let mut edits = Vec::new();
    let mut files = FileEdits::default();
    for diagnostic in (fixing.program.iter()).filter(|d| fix.fixes(d)) {
        diagnostics.push(diagnostic.clone());
        match edits_for {
            AllEdits::Here(add) => add(fixing, diagnostic, &mut edits),
            AllEdits::Files(add) => add(fixing, diagnostic, &mut files),
        }
    }
    if let AllEdits::Here(_) = edits_for {
        edits.sort_by_key(|e: &Edit| e.range.start());
        let mut last_end: Option<TextSize> = None;
        for edit in &mut edits {
            if let Some(end) = last_end
                && end > edit.range.start()
            {
                edit.range = TextRange::new(end, edit.range.end().max(end));
            }
            last_end = Some(edit.range.end());
        }
        files = here(fixing, edits);
    }
    Action {
        title: title.to_owned(),
        kind: ActionKind::QuickFix,
        diagnostics: Some(ActionDiagnostics::Program(diagnostics)),
        edits: Some(files),
        data: ActionData::Fix {
            id: fix.id,
            all: true,
        },
        preferred: Some(false),
        command: None,
    }
}

/// `isPreferredFix`: no other action outranks it, and for a fix there
/// can be only one of, no other of the same fix ties it.
fn is_preferred(actions: &[Action], i: usize) -> bool {
    let Some(mine) = Preferred::of(&actions[i].data) else {
        return false;
    };
    (actions.iter().enumerate()).all(|(j, other)| {
        j == i
            || Preferred::of(&other.data).is_none_or(|theirs| {
                theirs.priority < mine.priority
                    || (theirs.priority == mine.priority && !mine.only_one)
            })
    })
}

/// `ElmMakeDiagnostics.onCodeAction`: elm's suggested names.
fn elm_make_actions(path: &Path, i: usize, given: &Diagnosed) -> Vec<Action> {
    if given.source != Some(Source::Elm) {
        return Vec::new();
    }
    let message = &given.message;
    let suggested: Vec<String> = if [
        "NAMING ERROR",
        "BAD IMPORT",
        "UNKNOWN LICENSE",
        "UNKNOWN PACKAGE",
        "UNKNOWN EXPORT",
    ]
    .iter()
    .any(|p| message.starts_with(p))
    {
        // `^\s{4}#(.*)#$`, line by line.
        (message.lines())
            .filter_map(|line| {
                let rest = line
                    .get(4..)
                    .filter(|_| line[..4].chars().all(char::is_whitespace))?;
                let inner = rest.strip_prefix('#')?.strip_suffix('#')?;
                Some(inner.to_owned())
            })
            .collect()
    } else if message.starts_with("MODULE NAME MISMATCH")
        || message.starts_with("UNEXPECTED SYMBOL")
    {
        // `# -> #(.*)#$`, its first match.
        (message.lines())
            .find_map(|line| {
                let (_, rest) = line.split_once("# -> #")?;
                rest.strip_suffix('#').map(str::to_owned)
            })
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };
    (suggested.into_iter())
        .map(|name| Action {
            title: format!("Change to `{name}`"),
            kind: ActionKind::QuickFix,
            diagnostics: Some(ActionDiagnostics::Given(i)),
            edits: Some(FileEdits::of(
                path,
                vec![Edit {
                    range: given.range,
                    text: name,
                }],
            )),
            data: ActionData::None,
            preferred: None,
            command: None,
        })
        .collect()
}

/// The token at `offset` as elm-language-server's
/// `getNamedDescendantForPosition` finds it: after a name's character
/// the token that character is in, else the one starting there.
fn token_at(root: &SyntaxNode, offset: TextSize) -> Option<SyntaxToken> {
    let found = root.token_at_offset(offset);
    let before = found.clone().left_biased().and_then(|left| {
        let within = usize::from(offset.checked_sub(left.text_range().start())?);
        left.text().get(..within)?.chars().last()
    });
    if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
        found.left_biased()
    } else {
        found.right_biased()
    }
}

/// The name a value's definition starts with, when `token` is it.
fn definition_name(token: &SyntaxToken) -> Option<SyntaxNode> {
    let definition = token.parent()?;
    (token.kind() == SyntaxKind::LowerIdent
        && definition.kind() == SyntaxKind::Definition
        && name_of(&definition).as_ref() == Some(token))
    .then_some(definition)
}

/// `node`'s first own name, lower or upper case.
fn name_of(node: &SyntaxNode) -> Option<SyntaxToken> {
    (node.children_with_tokens())
        .filter_map(|e| e.into_token())
        .find(|t| matches!(t.kind(), SyntaxKind::LowerIdent | SyntaxKind::UpperIdent))
}

/// `edits` in the file the request is about.
fn here(fixing: &Fixing<'_>, edits: Vec<Edit>) -> FileEdits {
    FileEdits::of(fixing.context.path, edits)
}

/// `elmReviewFixAction`: elm-review's fix for each diagnostic the editor
/// sent that has one, as elm-language-server offers all of them for each.
fn review_actions(fixing: &Fixing<'_>, _range: TextRange) -> Vec<Offer> {
    (fixing.request.given.iter())
        .filter(|d| d.is(Code::ElmReview) && !d.fixes.is_empty())
        .map(|d| Offer::new("Apply elm-review fix", here(fixing, d.fixes.clone())))
        .collect()
}
