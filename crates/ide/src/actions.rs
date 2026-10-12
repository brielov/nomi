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

use std::path::{Path, PathBuf};

use nomi_base::{TextRange, TextSize};
use std::collections::BTreeMap;

use nomi_engine::index::Index;
use nomi_engine::{Snapshot, Store};
use nomi_project::Outline;
use nomi_syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

use crate::Edit;
use crate::completion::Context;
use crate::completion_general::{compare_ranking, importable};
use crate::problems::{Problem, Severity};
use crate::symbols::{child, code_range};

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
}

impl Diagnosed {
    fn of(problem: &Problem) -> Diagnosed {
        Diagnosed {
            code: problem.code.to_owned(),
            message: problem.message.clone(),
            range: problem.range,
            severity: problem.severity,
            source: problem.source.to_owned(),
            unnecessary: problem.unnecessary,
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
                        .push(package.clone());
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
    },
    /// Elm make's suggestions carry nothing.
    None,
}

/// A command the editor runs for an action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionCommand {
    /// Add `package` to the project.
    InstallPackage { package: String },
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
    actions: fn(&Fixing<'_>, TextRange) -> Vec<Offer>,
    all: Option<(&'static str, AllEdits)>,
}

/// A fix's action: its title, its edits, and the command it runs.
type Offer = (String, Vec<Edit>, Option<ActionCommand>);

/// A fix-all's edits for one diagnostic, added to those so far.
type AllEdits = fn(&Fixing<'_>, &Diagnosed, &mut Vec<Edit>);

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
        actions: import_actions,
        all: Some(("Add all missing imports", import_all)),
    },
    Fix {
        id: "add_type_annotation",
        codes: &["missing_type_annotation"],
        actions: annotation_actions,
        all: Some(("Add all missing type annotations", annotation_all)),
    },
    Fix {
        id: "install_package",
        codes: &["import_resolve"],
        actions: install_actions,
        all: None,
    },
];

/// What fixes read: the file, and its program diagnostics.
struct Fixing<'a> {
    context: Context<'a>,
    program: Vec<Diagnosed>,
    packages: &'a PackageModules,
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
        .root
        .first_child()
        .is_some_and(|n| n.kind() == SyntaxKind::Error)
    {
        return Vec::new();
    }
    let fixing = Fixing {
        context: Context {
            snapshot,
            index,
            module,
            path,
        },
        program: program_diagnostics(&module.text, request.problems),
        packages: request.packages,
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
        for fix in FIXES
            .iter()
            .filter(|f| f.codes.contains(&given.code.as_str()))
        {
            let found = (fix.actions)(&fixing, given.range);
            if found.is_empty() {
                continue;
            }
            for (title, edits, command) in found {
                results.push(Action {
                    title,
                    kind: ActionKind::QuickFix,
                    diagnostics: Some(ActionDiagnostics::Given(i)),
                    edits: Some(vec![(path.to_owned(), edits)]),
                    data: ActionData::Fix {
                        id: fix.id,
                        all: false,
                    },
                    preferred: None,
                    command,
                });
            }
            let others =
                (fixing.program.iter()).any(|d| d != given && fix.codes.contains(&d.code.as_str()));
            if let Some((title, edits_for)) = fix.all
                && others
            {
                results.push(fix_all(&fixing, fix, title, edits_for, path));
            }
        }
    }
    for (name, action, title, kind) in refactors(&fixing, range, request.given) {
        let edits = (!request.lazy)
            .then(|| refactor_edits_in(&fixing.context, action, range))
            .flatten();
        results.push(Action {
            title,
            kind,
            diagnostics: None,
            edits: edits.map(|edits| vec![(path.to_owned(), edits)]),
            data: ActionData::Refactor {
                name,
                action,
                range,
            },
            preferred: None,
            command: None,
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
fn program_diagnostics(text: &str, problems: &[Problem]) -> Vec<Diagnosed> {
    let mut found = Vec::new();
    for problem in problems.iter().filter(|p| p.source == "Elm") {
        if problem.code == "missing_value" {
            let name = &text[std::ops::Range::<usize>::from(problem.range)];
            found.push(Diagnosed {
                message: format!("No definition found for `{name}`."),
                ..Diagnosed::of(problem)
            });
        }
    }
    found.extend(
        (problems.iter())
            .filter(|p| p.code == "missing_type_annotation")
            .map(Diagnosed::of),
    );
    found.extend(
        (problems.iter())
            .filter(|p| p.source == "ElmLS")
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
    let mut diagnostics = Vec::new();
    for diagnostic in fixing
        .program
        .iter()
        .filter(|d| fix.codes.contains(&d.code.as_str()))
    {
        diagnostics.push(diagnostic.clone());
        edits_for(fixing, diagnostic, &mut edits);
    }
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
    Action {
        title: title.to_owned(),
        kind: ActionKind::QuickFix,
        diagnostics: Some(ActionDiagnostics::Program(diagnostics)),
        edits: Some(vec![(path.to_owned(), edits)]),
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

/// The name a missing-value diagnostic is about: its module as written,
/// empty when unqualified, and its last part.
fn missing_name(fixing: &Fixing<'_>, range: TextRange) -> (String, String) {
    let text = &fixing.context.module.text[std::ops::Range::<usize>::from(range)];
    match text.rsplit_once('.') {
        Some((module, name)) => (module.to_owned(), name.to_owned()),
        None => (String::new(), text.to_owned()),
    }
}

/// The imports that would bring the missing name at `range`: one that
/// exposes it, or for a qualified name, a module named so or ending so.
fn imports_for(fixing: &Fixing<'_>, range: TextRange) -> Vec<(String, Option<String>, Edit)> {
    let (target, name) = missing_name(fixing, range);
    let qualified = !target.is_empty();
    (importable(&fixing.context).into_iter())
        .filter(|p| {
            p.value == name
                && (!qualified
                    || if target.contains('.') {
                        p.module == target
                    } else {
                        p.module.ends_with(&target)
                    })
        })
        .map(|p| {
            let value =
                (!qualified).then(|| p.value_to_import.clone().unwrap_or_else(|| p.value.clone()));
            let edit = add_import(&fixing.context, &p.module, value.as_deref(), &target);
            (p.module, value, edit)
        })
        .collect()
}

/// `RefactorEditUtils.addImport`.
fn add_import(context: &Context<'_>, module: &str, value: Option<&str>, alias: &str) -> Edit {
    let mut edit = context.add_import(module, module);
    let alias = if !alias.is_empty() && alias != module {
        format!(" as {alias}")
    } else {
        String::new()
    };
    edit.text = match value {
        Some(value) => format!("import {module}{alias} exposing ({value})\n"),
        None => format!("import {module}{alias}\n"),
    };
    edit
}

fn import_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    let (target, _) = missing_name(fixing, range);
    (imports_for(fixing, range).into_iter())
        .map(|(module, value, edit)| {
            let alias = if !target.is_empty() && module != target {
                format!(" as \"{target}\"")
            } else {
                String::new()
            };
            let title = match value {
                Some(value) => format!("Import '{value}' from module \"{module}\"{alias}"),
                None => format!("Import module \"{module}\"{alias}"),
            };
            (title, vec![edit], None)
        })
        .collect()
}

fn import_all(fixing: &Fixing<'_>, diagnostic: &Diagnosed, edits: &mut Vec<Edit>) {
    if let Some((_, _, edit)) = imports_for(fixing, diagnostic.range).into_iter().next()
        && !edits.iter().any(|e| e.text == edit.text)
    {
        edits.push(edit);
    }
}

/// `installPackageCodeAction`: each package that has the missing
/// module, by rank, installed by a command rather than an edit.
fn install_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    let module = &fixing.context.module.text[std::ops::Range::<usize>::from(range)];
    let mut packages = (fixing.packages.0.get(module).cloned()).unwrap_or_default();
    packages.sort_by(|a, b| compare_ranking(a, b));
    (packages.into_iter())
        .map(|package| {
            let title = format!("Install package \"{package}\"");
            (
                title,
                Vec::new(),
                Some(ActionCommand::InstallPackage { package }),
            )
        })
        .collect()
}

/// The name token at `offset`, as tree-sitter's named descendant at a
/// position finds it: the token starting there, else the one it's in or
/// ends.
fn token_at(root: &SyntaxNode, offset: TextSize) -> Option<SyntaxToken> {
    let found = root.token_at_offset(offset);
    match found.clone().right_biased() {
        Some(right) if right.kind().is_trivia() => found.left_biased(),
        right => right,
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

/// `addTypeAnnotationCodeAction`'s edit: the inferred annotation above the
/// definition whose name is at `range`, at its column.
fn annotation_edits(context: &Context<'_>, range: TextRange) -> Option<Vec<Edit>> {
    let module = context.module;
    let token = token_at(&module.root, range.start())?;
    let definition = definition_name(&token)?;
    let ty = crate::hover::definition_type(context.index, module, &definition)?;
    let start = token.text_range().start();
    let text = &*module.text;
    let line_start = text[..usize::from(start)].rfind('\n').map_or(0, |i| i + 1);
    let column = text[line_start..usize::from(start)].chars().count();
    Some(vec![Edit {
        range: TextRange::empty(start),
        text: format!("{} : {ty}\n{}", token.text(), " ".repeat(column)),
    }])
}

fn annotation_actions(fixing: &Fixing<'_>, range: TextRange) -> Vec<Offer> {
    (annotation_edits(&fixing.context, range).into_iter())
        .map(|edits| ("Add inferred annotation".to_owned(), edits, None))
        .collect()
}

fn annotation_all(fixing: &Fixing<'_>, diagnostic: &Diagnosed, edits: &mut Vec<Edit>) {
    if let Some(found) = annotation_edits(&fixing.context, diagnostic.range)
        && !edits.iter().any(|e| e.text == found[0].text)
    {
        edits.extend(found);
    }
}

/// The refactors at `range`, in elm-language-server's registration
/// order: name, action, title and kind.
fn refactors(
    fixing: &Fixing<'_>,
    range: TextRange,
    given: &[Diagnosed],
) -> Vec<(&'static str, &'static str, String, ActionKind)> {
    let mut found = Vec::new();
    let root = &fixing.context.module.root;
    let text = &*fixing.context.module.text;
    let Some(token) = token_at(root, range.start()) else {
        return found;
    };
    // An unannotated value's inferred annotation, where no diagnostic
    // offers it already.
    let annotated = definition_name(&token)
        .and_then(|d| d.parent())
        .map(|d| child(&d, SyntaxKind::Signature).is_some());
    if annotated == Some(false) && !given.iter().any(|d| d.code == "missing_type_annotation") {
        found.push((
            "add_type_annotation",
            "add_type_annotation",
            "Add inferred annotation".to_owned(),
            ActionKind::RefactorExtract,
        ));
    }
    let parent = token.parent();
    let parent_kind = parent.as_ref().map(SyntaxNode::kind);
    let named = parent.as_ref().and_then(name_of).as_ref() == Some(&token);
    let in_let = token
        .parent_ancestors()
        .any(|a| a.kind() == SyntaxKind::LetExpr);
    let expose = "expose_unexpose";
    // A value's name, or what tree-sitter-elm takes for one: the whole
    // type of a signature, whose punctuation is its own.
    let function = if token.kind() == SyntaxKind::LowerIdent
        && named
        && matches!(
            parent_kind,
            Some(SyntaxKind::Signature | SyntaxKind::Definition)
        ) {
        Some(token.text().to_owned())
    } else {
        signature_type(&token).map(|ty| written(text, &ty).to_owned())
    };
    if let Some(function) = function
        && !in_let
    {
        if exposes(root, text, &function, true) {
            found.push((
                expose,
                "unexpose_function",
                "Unexpose Function".to_owned(),
                ActionKind::Refactor,
            ));
        } else {
            found.push((
                expose,
                "expose_function",
                "Expose Function".to_owned(),
                ActionKind::Refactor,
            ));
        }
    }
    if token.kind() == SyntaxKind::UpperIdent
        && named
        && matches!(
            parent_kind,
            Some(SyntaxKind::TypeAliasDecl | SyntaxKind::CustomTypeDecl)
        )
    {
        let alias = if parent_kind == Some(SyntaxKind::TypeAliasDecl) {
            " Alias"
        } else {
            ""
        };
        if exposes(root, text, token.text(), false) {
            found.push((
                expose,
                "unexpose_type",
                format!("Unexpose Type{alias}"),
                ActionKind::Refactor,
            ));
        } else {
            found.push((
                expose,
                "expose_type",
                format!("Expose Type{alias}"),
                ActionKind::Refactor,
            ));
            if parent_kind == Some(SyntaxKind::CustomTypeDecl) {
                found.push((
                    expose,
                    "expose_type_with_variants",
                    "Expose Type with Variants".to_owned(),
                    ActionKind::Refactor,
                ));
            }
        }
    }
    found
}

/// The type of a signature, when `token` is punctuation tree-sitter-elm
/// gives to its outermost `type_expression`: the arrows of a function
/// type written there, and the parentheses around it or its parts.
fn signature_type(token: &SyntaxToken) -> Option<SyntaxNode> {
    if token.kind().is_trivia()
        || matches!(
            token.kind(),
            SyntaxKind::LowerIdent | SyntaxKind::UpperIdent
        )
    {
        return None;
    }
    let parent = token.parent()?;
    let up = parent.parent()?;
    let top = match parent.kind() {
        SyntaxKind::FunctionType | SyntaxKind::ParenType if up.kind() == SyntaxKind::Signature => {
            parent
        }
        SyntaxKind::ParenType
            if up.kind() == SyntaxKind::FunctionType
                && up
                    .parent()
                    .is_some_and(|s| s.kind() == SyntaxKind::Signature) =>
        {
            up
        }
        _ => return None,
    };
    Some(top)
}

/// A refactor's edits, as resolving it gives them.
pub fn refactor_edits(
    index: &Index,
    snapshot: &Snapshot,
    path: &Path,
    action: &str,
    range: TextRange,
) -> Option<Vec<Edit>> {
    let module = index.module(path)?;
    let context = Context {
        snapshot,
        index,
        module,
        path,
    };
    refactor_edits_in(&context, action, range)
}

fn refactor_edits_in(context: &Context<'_>, action: &str, range: TextRange) -> Option<Vec<Edit>> {
    let module = context.module;
    match action {
        "add_type_annotation" => annotation_edits(context, range),
        "unexpose_function"
        | "unexpose_type"
        | "expose_function"
        | "expose_type"
        | "expose_type_with_variants" => {
            let token = token_at(&module.root, range.start())?;
            let name = match signature_type(&token) {
                Some(ty) => written(&module.text, &ty).to_owned(),
                None => token.text().to_owned(),
            };
            let edit = match action {
                "expose_function" | "expose_type" => {
                    expose_edit(&module.root, &module.text, &name, false)
                }
                "expose_type_with_variants" => expose_edit(&module.root, &module.text, &name, true),
                _ => unexpose_edit(&module.root, &module.text, &name),
            };

            edit.map(|edit| vec![edit])
        }
        _ => None,
    }
}

/// Whether the module exposes `name` (`isExposedFunctionOrPort`,
/// `isExposedTypeOrTypeAlias`): by its exposing list's text.
fn exposes(root: &SyntaxNode, text: &str, name: &str, function: bool) -> bool {
    crate::lens::exposes(root, text, name, function)
}

/// The exposed values and types of the module's exposing list
/// (`getModuleExposingListNodes`).
fn exposed_nodes(root: &SyntaxNode) -> Vec<SyntaxNode> {
    (root.children())
        .find(|n| n.kind() == SyntaxKind::ModuleHeader)
        .and_then(|header| child(&header, SyntaxKind::ExposingList))
        .map(|list| {
            (list.descendants())
                .filter(|n| matches!(n.kind(), SyntaxKind::ExposedValue | SyntaxKind::ExposedType))
                .collect()
        })
        .unwrap_or_default()
}

fn written<'t>(text: &'t str, node: &SyntaxNode) -> &'t str {
    code_range(node).map_or("", |r| &text[std::ops::Range::<usize>::from(r)])
}

/// The edit of the file at `path` that exposes `name`, or unexposes it:
/// elm-language-server's `elm/expose` and `elm/unexpose` requests.
pub fn exposing_edit(snapshot: &Snapshot, path: &Path, name: &str, expose: bool) -> Option<Edit> {
    let text = snapshot.read(path).ok()?;
    let root = nomi_syntax::parse_module(&text).syntax();
    if expose {
        expose_edit(&root, &text, name, false)
    } else {
        unexpose_edit(&root, &text, name)
    }
}

/// `exposeValueInModule`: `, name` after the last exposed, unless the
/// list is `..` or exposes it already.
pub(crate) fn expose_edit(
    root: &SyntaxNode,
    text: &str,
    name: &str,
    with_variants: bool,
) -> Option<Edit> {
    let nodes = exposed_nodes(root);
    let name = if with_variants {
        format!("{name}(..)")
    } else {
        name.to_owned()
    };
    if nodes.iter().any(|n| written(text, n) == name) {
        return None;
    }
    let last = code_range(nodes.last()?)?;
    Some(Edit {
        range: TextRange::empty(last.end()),
        text: format!(", {name}"),
    })
}

/// `unexposedValueInModule`: `name` out of the exposing list, with the
/// comma between it and its neighbor, unless it's the only one.
pub(crate) fn unexpose_edit(root: &SyntaxNode, text: &str, name: &str) -> Option<Edit> {
    let nodes = exposed_nodes(root);
    if nodes.len() <= 1 {
        return None;
    }
    let node = (nodes.iter()).find(|n| {
        let written = written(text, n);
        written == name || written == format!("{name}(..)")
    })?;
    let range = code_range(node)?;
    // Its siblings as tree-sitter has them: no whitespace.
    let siblings: Vec<SyntaxElement> = (node.parent()?.children_with_tokens())
        .filter(|e| !matches!(e.kind(), SyntaxKind::Whitespace | SyntaxKind::Newline))
        .collect();
    let at = siblings.iter().position(|e| e.as_node() == Some(node))?;
    let is = |i: Option<usize>, kind: SyntaxKind| {
        i.and_then(|i| siblings.get(i))
            .is_some_and(|e| e.kind() == kind)
    };
    let mut start = range.start();
    let mut end = range.end();
    if is(at.checked_sub(1), SyntaxKind::Comma) && is(Some(at + 1), SyntaxKind::RParen) {
        start = siblings[at - 1].text_range().start();
    }
    if is(Some(at + 1), SyntaxKind::Comma)
        && let Some(after) = siblings.get(at + 2)
    {
        end = after.text_range().start();
    }
    Some(Edit {
        range: TextRange::new(start, end),
        text: String::new(),
    })
}
