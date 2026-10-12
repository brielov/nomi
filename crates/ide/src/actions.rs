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
use nomi_syntax::SyntaxKind;

use crate::Edit;
use crate::completion::Context;
use crate::completion_general::{compare_ranking, importable};
use crate::problems::{Problem, Severity};

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
    preferred: Option<Preferred>,
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

/// The fixes, in elm-language-server's registration order.
const FIXES: &[Fix] = &[
    Fix {
        id: "import",
        codes: &["missing_value"],
        preferred: Some(Preferred {
            priority: 1,
            only_one: true,
        }),
        actions: import_actions,
        all: Some(("Add all missing imports", import_all)),
    },
    Fix {
        id: "install_package",
        codes: &["import_resolve"],
        preferred: None,
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
    for (i, given) in request.given.iter().enumerate() {
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
    let priority = |action: &Action| match action.data {
        ActionData::Fix { id, all: false } => (FIXES.iter())
            .find(|f| f.id == id)
            .and_then(|f| f.preferred)
            .map(|p| (id, p)),
        _ => None,
    };
    let Some((id, mine)) = priority(&actions[i]) else {
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
            Some((other_id, _)) => !(mine.only_one && other_id == id),
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
