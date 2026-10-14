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

use std::cell::OnceCell;
use std::path::{Path, PathBuf};

use nomi_base::{TextRange, TextSize};
use std::collections::BTreeMap;

use nomi_compiler::types::{ExprTypes, error};
use nomi_engine::index::Index;
use nomi_engine::{Snapshot, Store};
use nomi_project::Outline;
use nomi_syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

use crate::Edit;
use crate::completion::Context;
use crate::completion_general::{compare_ranking, importable};
use crate::problems::{Code, Problem, Severity, Source};
use crate::tree::{child, code_range};

mod extract;
mod import;
mod missing;
mod moving;
mod refactor;
mod unused;
mod usage;

use import::*;
use missing::*;
use refactor::*;
use unused::*;
use usage::*;

pub use moving::{Destination, MoveEdit, move_destinations, move_function};
pub use refactor::{exposing_edit, refactor_edits};

/// A diagnostic as code actions see it: one the editor sends, or one of
/// the program's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnosed {
    pub code: String,
    pub message: String,
    pub range: TextRange,
    pub severity: Severity,
    /// `Elm` or `ElmLS`.
    pub source: String,
    /// Whether it marks code nothing needs.
    pub unnecessary: bool,
    /// The edits that fix it, which elm-review gives.
    pub fixes: Vec<Edit>,
}

impl Diagnosed {
    fn of(problem: &Problem) -> Diagnosed {
        Diagnosed {
            code: problem.code.as_str().to_owned(),
            message: problem.message.clone(),
            range: problem.range,
            severity: problem.severity,
            source: problem.code.source().as_str().to_owned(),
            unnecessary: problem.unnecessary,
            fixes: problem.fixes.clone(),
        }
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
    /// Whether the editor resolves a refactor's edits later.
    pub lazy: bool,
    /// Whether the editor moves functions, asking where to.
    pub move_function: bool,
}

/// `ElmPackageCache`'s modules: the packages downloaded whose newest
/// release exposes each module.
#[derive(Debug, Default)]
pub struct PackageModules(BTreeMap<String, Vec<String>>);

impl PackageModules {
    pub fn read(store: &Store) -> PackageModules {
        let mut modules: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (package, sources) in store.newest().unwrap_or_default() {
            let Ok(text) = std::fs::read(sources.join("elm.json")) else {
                continue;
            };
            if let Ok((Outline::Pkg(outline), _)) = Outline::detect(&text) {
                for module in outline.exposed.modules() {
                    modules
                        .entry(module.to_string())
                        .or_default()
                        .push(package.to_string());
                }
            }
        }
        PackageModules(modules)
    }
}

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
    /// A fix, by its id, and whether it fixes every diagnostic.
    Fix { id: &'static str, all: bool },
    /// A refactor, by its name and action, at the range it was asked
    /// about, which resolving it reads again.
    Refactor {
        name: &'static str,
        action: &'static str,
        range: TextRange,
        /// Where the editor starts renaming what it made.
        rename: Option<RenameAt>,
    },
    /// Elm make's suggestions carry nothing.
    None,
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
pub(super) struct Refactor {
    pub name: &'static str,
    pub action: &'static str,
    pub title: String,
    pub kind: ActionKind,
    pub command: Option<ActionCommand>,
}

impl Refactor {
    pub(crate) fn new(
        name: &'static str,
        action: &'static str,
        title: impl Into<String>,
        kind: ActionKind,
    ) -> Refactor {
        Refactor {
            name,
            action,
            title: title.into(),
            kind,
            command: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub title: String,
    pub kind: ActionKind,
    pub diagnostics: Option<ActionDiagnostics>,
    /// Each file's edits; none for an action whose edit comes later.
    pub edits: Option<Vec<(PathBuf, Vec<Edit>)>>,
    pub data: ActionData,
    /// Whether the editor should prefer it; none for elm make's.
    pub preferred: Option<bool>,
    pub command: Option<ActionCommand>,
}

/// A fix's registration: `ICodeActionRegistration`.
struct Fix {
    id: &'static str,
    codes: &'static [&'static str],
    /// elm's reports it answers too, where elm-language-server answers
    /// its own checker's (DECISIONS 2026-10-12).
    reports: Option<fn(&Diagnosed) -> bool>,
    actions: fn(&Fixing<'_>, TextRange) -> Vec<Offer>,
    all: Option<(&'static str, AllEdits)>,
}

impl Fix {
    /// Whether it fixes `diagnostic`: by its code, or as elm's report.
    fn applies(&self, diagnostic: &Diagnosed) -> bool {
        self.codes.contains(&diagnostic.code.as_str())
            || (self.reports).is_some_and(|is| diagnostic.source == "Elm" && is(diagnostic))
    }
}

/// A fix's action: its title, each file's edits, and the command it runs.
type Offer = (String, FileEdits, Option<ActionCommand>);

/// Each file's edits, in order.
type FileEdits = Vec<(PathBuf, Vec<Edit>)>;

/// A fix-all's edits for one diagnostic, added to those so far: in the
/// file, sorted at the end, or in any files, each file's in order.
#[derive(Clone, Copy)]
enum AllEdits {
    Here(fn(&Fixing<'_>, &Diagnosed, &mut Vec<Edit>)),
    Files(fn(&Fixing<'_>, &Diagnosed, &mut FileEdits)),
}

#[derive(Clone, Copy)]
struct Preferred {
    priority: u8,
    only_one: bool,
}

/// The preferred fixes and refactors, by id or name: elm-language-server
/// keeps them in one map, so a fix and a refactor of one name share it.
fn preferred(key: &str) -> Option<Preferred> {
    let (priority, only_one) = match key {
        "import" => (1, true),
        "expose_unexpose" => (2, false),
        "add_type_annotation" => (3, false),
        _ => return None,
    };
    Some(Preferred { priority, only_one })
}

/// The fixes, in elm-language-server's registration order.
const FIXES: &[Fix] = &[
    Fix {
        id: "import",
        codes: &["missing_value"],
        reports: None,
        actions: import_actions,
        all: Some(("Add all missing imports", AllEdits::Here(import_all))),
    },
    Fix {
        id: "make_declaration_from_usage",
        codes: &["missing_value"],
        reports: None,
        actions: local_function_actions,
        all: Some((
            "Create all missing local functions",
            AllEdits::Here(local_function_all),
        )),
    },
    Fix {
        id: "make_external_declaration_from_usage",
        codes: &["missing_value"],
        reports: None,
        actions: external_function_actions,
        all: Some((
            "Create all missing external functions",
            AllEdits::Files(external_function_all),
        )),
    },
    Fix {
        id: "add_type_annotation",
        codes: &["missing_type_annotation"],
        reports: None,
        actions: annotation_actions,
        all: Some((
            "Add all missing type annotations",
            AllEdits::Here(annotation_all),
        )),
    },
    Fix {
        id: "add_missing_record_field",
        codes: &["record_field"],
        reports: Some(is_missing_field),
        actions: record_field_actions,
        all: Some((
            "Create all missing record fields",
            AllEdits::Files(record_field_all),
        )),
    },
    Fix {
        id: "add_missing_case_branches",
        codes: &["incomplete_case_pattern"],
        reports: Some(is_missing_patterns),
        actions: case_branch_actions,
        all: Some((
            "Add all missing case branches",
            AllEdits::Here(case_branch_all),
        )),
    },
    Fix {
        id: "elm_review_fix",
        codes: &["elm_review"],
        reports: None,
        actions: review_actions,
        all: None,
    },
    Fix {
        id: "add_missing_union_type",
        codes: &["missing_value"],
        reports: None,
        actions: union_actions,
        all: Some((
            "Create all missing union constructors",
            AllEdits::Files(union_all),
        )),
    },
    Fix {
        id: "add_new_function_parameter",
        codes: &["missing_value"],
        reports: None,
        actions: parameter_actions,
        all: None,
    },
    Fix {
        id: "remove_unused",
        codes: UNUSED,
        reports: None,
        actions: unused_actions,
        all: Some((
            "Remove all reported unused code",
            AllEdits::Here(unused_all),
        )),
    },
    Fix {
        id: "remove_unused_parameter",
        codes: &["unused_pattern"],
        reports: None,
        actions: parameter_removal_actions,
        all: None,
    },
    Fix {
        id: "install_package",
        codes: &["import_resolve"],
        reports: None,
        actions: install_actions,
        all: None,
    },
];

/// The module with its missing values made `Debug.todo`: its types,
/// and where each missing value went, by its range before.
struct Usage {
    types: ExprTypes,
    moved: Vec<(TextRange, TextRange)>,
}

/// What fixes read: the file, and its program diagnostics.
struct Fixing<'a> {
    context: Context<'a>,
    program: Vec<Diagnosed>,
    packages: &'a PackageModules,
    /// The diagnostics the editor sent.
    given: &'a [Diagnosed],
    /// Whether the editor moves functions.
    move_function: bool,
    /// What a fix-all removes from each import so far.
    removed_imports: std::cell::RefCell<RemovedImports>,
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
        program: program_diagnostics(&module.root(), &module.text, request.problems),
        packages: request.packages,
        given: request.given,
        move_function: request.move_function,
        removed_imports: Default::default(),
        usage: OnceCell::new(),
    };
    let mut results: Vec<Action> = Vec::new();
    // Each diagnostic's range becomes the request's, so refactors see the
    // last one's.
    let mut range = request.range;
    for (i, given) in request.given.iter().enumerate() {
        if given.code.is_empty() {
            continue;
        }
        range = given.range;
        for fix in FIXES.iter().filter(|f| f.applies(given)) {
            let found = (fix.actions)(&fixing, given.range);
            if found.is_empty() {
                continue;
            }
            for (title, edits, command) in found {
                results.push(Action {
                    title,
                    kind: ActionKind::QuickFix,
                    diagnostics: Some(ActionDiagnostics::Given(i)),
                    edits: Some(edits),
                    data: ActionData::Fix {
                        id: fix.id,
                        all: false,
                    },
                    preferred: None,
                    command,
                });
            }
            // Another diagnostic it fixes. elm's report and the checker's
            // form of it nomi writes are one.
            let same = |d: &Diagnosed| {
                d == given || (d.range == given.range && fix.reports.is_some_and(|is| is(given)))
            };
            let others =
                (fixing.program.iter()).any(|d| !same(d) && fix.codes.contains(&d.code.as_str()));
            if let Some((title, edits_for)) = fix.all
                && others
            {
                results.push(fix_all(&fixing, fix, title, edits_for, path));
            }
        }
    }
    for refactor in refactors(&fixing, range, request.given) {
        let edits = (!request.lazy)
            .then(|| refactor_edits_in(&fixing.context, refactor.action, range))
            .flatten();
        let rename = edits.as_ref().and_then(|(_, rename)| *rename);
        results.push(Action {
            title: refactor.title,
            kind: refactor.kind,
            diagnostics: None,
            edits: edits.map(|(edits, _)| vec![(path.to_owned(), edits)]),
            data: ActionData::Refactor {
                name: refactor.name,
                action: refactor.action,
                range,
                rename,
            },
            preferred: None,
            command: refactor.command,
        });
    }
    let preferred: Vec<Option<bool>> = (0..results.len())
        .map(|i| Some(is_preferred(&results, i)))
        .collect();
    for (action, preferred) in results.iter_mut().zip(preferred) {
        action.preferred = preferred;
    }
    for (i, given) in request.given.iter().enumerate() {
        results.extend(elm_make_actions(path, i, given));
    }
    results
}

/// `getDiagnostics`: the checker's, its suggestions, then the lint rules'.
fn program_diagnostics(root: &SyntaxNode, text: &str, problems: &[Problem]) -> Vec<Diagnosed> {
    let mut found = Vec::new();
    for problem in problems.iter().filter(|p| p.code.source() == Source::Elm) {
        let elm = Diagnosed::of(problem);
        if problem.code == Code::MissingValue {
            let name = &text[std::ops::Range::<usize>::from(problem.range)];
            found.push(Diagnosed {
                message: format!("No definition found for `{name}`."),
                ..elm
            });
        } else if is_missing_patterns(&elm) {
            let missing: String = (missing_patterns(&elm.message).iter())
                .map(|p| format!("  {p}\n"))
                .collect();
            found.push(Diagnosed {
                code: "incomplete_case_pattern".to_owned(),
                message: format!(
                    "This `case` does not have branches for all possibilities.\nMissing possibilities include:\n{missing}"
                ),
                ..elm
            });
        } else if is_missing_field(&elm)
            && let Some((_, field)) = missing_field(root, &elm)
        {
            found.push(Diagnosed {
                code: "record_field".to_owned(),
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
fn fix_all(
    fixing: &Fixing<'_>,
    fix: &Fix,
    title: &str,
    edits_for: AllEdits,
    path: &Path,
) -> Action {
    let mut edits = Vec::new();
    let mut files: Vec<(PathBuf, Vec<Edit>)> = Vec::new();
    let mut diagnostics = Vec::new();
    for diagnostic in (fixing.program.iter()).filter(|d| fix.codes.contains(&d.code.as_str())) {
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
        files = vec![(path.to_owned(), edits)];
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
    let priority = |action: &Action| {
        let key = match action.data {
            ActionData::Fix { id, all: false } => id,
            ActionData::Refactor { name, .. } => name,
            _ => return None,
        };
        preferred(key).map(|p| (key, p))
    };
    let Some((key, mine)) = priority(&actions[i]) else {
        return false;
    };
    actions.iter().enumerate().all(|(j, other)| {
        if j == i {
            return true;
        }
        match priority(other) {
            None => true,
            Some((_, theirs)) if theirs.priority < mine.priority => true,
            Some((_, theirs)) if theirs.priority > mine.priority => false,
            Some((other_key, _)) => !(mine.only_one && other_key == key),
        }
    })
}

/// `ElmMakeDiagnostics.onCodeAction`: elm's suggested names.
fn elm_make_actions(path: &Path, i: usize, given: &Diagnosed) -> Vec<Action> {
    if given.source != "Elm" {
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
            edits: Some(vec![(
                path.to_owned(),
                vec![Edit {
                    range: given.range,
                    text: name,
                }],
            )]),
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
    // `getNamedDescendantForPosition`: after a name's character, the name
    // that ends there.
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

fn name_of(node: &SyntaxNode) -> Option<SyntaxToken> {
    (node.children_with_tokens())
        .filter_map(|e| e.into_token())
        .find(|t| matches!(t.kind(), SyntaxKind::LowerIdent | SyntaxKind::UpperIdent))
}

/// `edits` in the file the request is about.
fn here(fixing: &Fixing<'_>, edits: Vec<Edit>) -> Vec<(PathBuf, Vec<Edit>)> {
    vec![(fixing.context.path.to_owned(), edits)]
}

/// `Utils.mergeChanges`: each file's edits after those it has.
fn merge(files: &mut FileEdits, more: FileEdits) {
    for (path, edits) in more {
        match files.iter_mut().find(|(p, _)| *p == path) {
            Some((_, existing)) => existing.extend(edits),
            None => files.push((path, edits)),
        }
    }
}

/// `elmReviewFixAction`: elm-review's fix for each diagnostic the editor
/// sent that has one, as elm-language-server offers all of them for each.
fn review_actions(fixing: &Fixing<'_>, _range: TextRange) -> Vec<Offer> {
    (fixing.given.iter())
        .filter(|d| d.code == "elm_review" && !d.fixes.is_empty())
        .map(|d| {
            (
                "Apply elm-review fix".to_owned(),
                here(fixing, d.fixes.clone()),
                None,
            )
        })
        .collect()
}
