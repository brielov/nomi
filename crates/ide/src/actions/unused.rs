//! Fixes for the lint rules' unused code (`removeUnusedCodeAction.ts`):
//! imports, aliases, values, constructors, type aliases and patterns,
//! and unused parameters with the arguments every call passes them
//! (`removeUnusedParameterCodeAction.ts`).

use std::collections::{BTreeMap, BTreeSet};

use nomi_base::{TextRange, TextSize};
use nomi_project::Outline;
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

use super::refactor::{Commas, remove_exposed, with_commas};
use super::{Diagnosed, DiagnosticCode, Fixing, Offer, here, name_of, token_at};
use crate::completion::Context;
use crate::problems::Code;
use crate::tree::{
    child, code_range, declaration_range, last_child_token, variant_range, words, written,
};
use crate::{Edit, FileEdits, next_line};

/// For each diagnostic the editor sent, the action that removes what it
/// reports, as elm-language-server offers all of them for each one.
pub(super) fn unused_actions(fixing: &Fixing<'_>, _range: TextRange) -> Vec<Offer> {
    (fixing.request.given.iter())
        .filter_map(|d| {
            let (title, edits) = unused_edits(fixing, d, None);
            Some(Offer::new(title?, here(fixing, edits)))
        })
        .collect()
}

pub(super) fn unused_all(fixing: &Fixing<'_>, diagnostic: &Diagnosed, edits: &mut Vec<Edit>) {
    let mut imports = fixing.removed_imports.borrow_mut();
    edits.extend(unused_edits(fixing, diagnostic, Some(&mut imports)).1);
}

/// The values a fix-all removes from each import so far, by module.
pub(super) type RemovedImports = BTreeMap<String, BTreeSet<String>>;

/// `getEditsForDiagnostic`: what removing the unused code `diagnostic`
/// reports takes, and its title where it has one: a fix-all's edits may
/// have none.
fn unused_edits(
    fixing: &Fixing<'_>,
    diagnostic: &Diagnosed,
    removed: Option<&mut RemovedImports>,
) -> (Option<String>, Vec<Edit>) {
    let module = fixing.context.module;
    let (root, text) = (&module.root(), &*module.text);
    let range = diagnostic.range;
    let Some(token) = token_at(root, range.start()) else {
        return (None, Vec::new());
    };
    // What a removal of `range`, where there's one, offers.
    let removal = |title: String, range: Option<TextRange>| match range {
        Some(range) => (Some(title), vec![Edit::delete(range)]),
        None => (None, Vec::new()),
    };
    let DiagnosticCode::Known(code) = diagnostic.code else {
        return (None, Vec::new());
    };
    match code {
        Code::UnusedImport => {
            let name = (token.parent_ancestors())
                .find(|a| a.kind() == SyntaxKind::Import)
                .and_then(|i| child(&i, SyntaxKind::ModuleName))
                .map_or_else(|| token.text().to_owned(), |n| words(&n));
            let range = TextRange::new(range.start(), next_line(text, range.end()));
            (
                Some(format!("Remove unused import `{name}`")),
                vec![Edit::delete(range)],
            )
        }
        Code::UnusedAlias => {
            let name = token_at(root, range.end())
                .filter(|t| t.kind() == SyntaxKind::UpperIdent)
                .or_else(|| root.token_at_offset(range.end()).left_biased());
            let name = name.map(|t| t.text().to_owned()).unwrap_or_default();
            let range = TextRange::new(range.start() - TextSize::new(1), range.end());
            (
                Some(format!("Remove unused alias `{name}`")),
                vec![Edit::delete(range)],
            )
        }
        Code::UnusedTopLevel => {
            let title = format!("Remove function `{}`", token.text());
            removal(title, remove_function(&token))
        }
        Code::UnusedPattern => {
            let field = token.parent().filter(|p| {
                p.kind() == SyntaxKind::FieldPattern
                    && p.parent()
                        .is_some_and(|r| r.kind() == SyntaxKind::RecordPattern)
            });
            let edit = match field {
                Some(field) => with_commas(&field, Commas::BeforeElseAfter).map(Edit::delete),
                None => Some(Edit {
                    range,
                    text: "_".to_owned(),
                }),
            };
            let title = format!("Fix unused pattern `{}`", token.text());
            (Some(title), edit.into_iter().collect())
        }
        Code::UnusedValueConstructor => {
            let title = format!("Remove unused value constructor `{}`", token.text());
            removal(title, remove_variant(&token))
        }
        Code::UnusedTypeAlias => {
            let alias = (token.parent_ancestors()).find(|a| a.kind() == SyntaxKind::TypeAliasDecl);
            let title = format!("Remove unused type alias `{}`", token.text());
            removal(title, alias.and_then(|a| code_range(&a)))
        }
        Code::UnusedImportedValue => imported_value_edits(fixing, &token, removed),
        // `getEditsForDiagnostic` has no case for the others, an unused
        // incoming port among them.
        Code::UnusedIncomingPort
        | Code::MissingValue
        | Code::ImportResolve
        | Code::ElmMake
        | Code::MissingTypeAnnotation
        | Code::RecordField
        | Code::IncompleteCasePattern
        | Code::ElmReview
        | Code::MapNothingToNothing
        | Code::BooleanCaseExpr
        | Code::DropConcatOfLists
        | Code::DropConsOfItemAndList
        | Code::UseConsOverConcat
        | Code::SingleFieldRecord
        | Code::UnnecessaryListConcat
        | Code::UnnecessaryPortModule
        | Code::NoUncurriedPrefix => (None, Vec::new()),
    }
}

/// An import's exposed value or type removed, or its whole list where a
/// fix-all removes every one.
fn imported_value_edits(
    fixing: &Fixing<'_>,
    token: &SyntaxToken,
    removed: Option<&mut RemovedImports>,
) -> (Option<String>, Vec<Edit>) {
    let text = &*fixing.context.module.text;
    let Some(import) = token.parent_ancestors().find_map(ast::Import::cast) else {
        return (None, Vec::new());
    };
    let Some(module) = import.module_name().map(|n| words(n.syntax())) else {
        return (None, Vec::new());
    };
    let (list, exposed) = import_exposing(&import);
    let all: Vec<&str> = exposed.iter().map(|n| written(text, n)).collect();
    let name = token.text();
    // What a fix-all has removed from the import, this one too.
    let gone = removed.map(|removed| {
        let gone = removed.entry(module).or_default();
        gone.insert(name.to_owned());
        &*gone
    });
    // Every value of the list goes: the list goes.
    if let Some(gone) = gone
        && all.iter().all(|v| gone.contains(*v))
        && let Some(edit) = list.as_ref().and_then(without_list)
    {
        return (None, vec![edit]);
    }
    let Some(edit) = remove_from_import(text, list.as_ref(), &exposed, name, Commas::Nearest)
    else {
        return (None, Vec::new());
    };
    // The values a fix-all removes from a list's end: the comma before the
    // first of them goes too.
    if let Some(gone) = gone
        && all.len() > 2
        && let Some(used) = all.iter().rev().position(|v| !gone.contains(*v))
        && used >= 2
        && let Some(with_comma) = remove_from_import(
            text,
            list.as_ref(),
            &exposed,
            all[all.len() - used],
            Commas::Before,
        )
    {
        return (None, vec![with_comma, edit]);
    }
    // elm-language-server says "type" only where its node is the exposed
    // type itself, and the diagnostic starts at the name.
    (Some(format!("Remove unused value `{name}`")), vec![edit])
}

/// `import`'s exposing list, and the values and types it exposes.
pub(super) fn import_exposing(import: &ast::Import) -> (Option<SyntaxNode>, Vec<SyntaxNode>) {
    let list = import.exposing_list().map(|l| l.syntax().clone());
    let exposed = (list.iter())
        .flat_map(|l| l.children())
        .filter(|n| matches!(n.kind(), SyntaxKind::ExposedValue | SyntaxKind::ExposedType))
        .collect();
    (list, exposed)
}

/// `removeValueFromImport`: the whole exposing list when `name` is all
/// it exposes, else `name` out of it.
pub(super) fn remove_from_import(
    text: &str,
    list: Option<&SyntaxNode>,
    exposed: &[SyntaxNode],
    name: &str,
    commas: Commas,
) -> Option<Edit> {
    if exposed.len() == 1 && written(text, &exposed[0]) == name {
        return without_list(list?);
    }
    remove_exposed(exposed, text, name, commas)
}

/// `removeImportExposingList`: from the space before `exposing` to the
/// list's end.
fn without_list(list: &SyntaxNode) -> Option<Edit> {
    let range = code_range(list)?;
    let start = range.start() - TextSize::new(1);
    Some(Edit::delete(TextRange::new(start, range.end())))
}

/// The element before `element` that isn't whitespace, as tree-sitter's
/// previous sibling.
fn previous_code(element: &SyntaxElement) -> Option<SyntaxToken> {
    let first = match element {
        SyntaxElement::Node(node) => node.first_token()?,
        SyntaxElement::Token(token) => token.clone(),
    };
    std::iter::successors(first.prev_token(), |t| t.prev_token())
        .find(|t| !matches!(t.kind(), SyntaxKind::Whitespace | SyntaxKind::Newline))
}

/// `removeFunction`: the value whose name is `token`, with its signature
/// and a block comment right before them.
fn remove_function(token: &SyntaxToken) -> Option<TextRange> {
    let definition = token
        .parent_ancestors()
        .find(|a| a.kind() == SyntaxKind::Definition)?;
    let declaration = definition.parent()?;
    let end = code_range(&definition)?.end();
    let mut start = code_range(&definition)?.start();
    let is_block =
        |t: &SyntaxToken| matches!(t.kind(), SyntaxKind::BlockComment | SyntaxKind::DocComment);
    match child(&declaration, SyntaxKind::Signature) {
        Some(signature)
            if previous_code(&SyntaxElement::Node(definition.clone()))
                .is_some_and(|t| signature.text_range().contains_range(t.text_range())) =>
        {
            start = code_range(&signature)?.start();
            if let Some(comment) = previous_code(&SyntaxElement::Node(signature)).filter(is_block) {
                start = comment.text_range().start();
            }
        }
        _ => {
            if let Some(comment) = previous_code(&SyntaxElement::Node(definition)).filter(is_block)
            {
                start = comment.text_range().start();
            }
        }
    }
    Some(TextRange::new(start, end))
}

/// `removeTypeValue`: a constructor of a custom type, or the whole type
/// when it's the only one.
fn remove_variant(token: &SyntaxToken) -> Option<TextRange> {
    let declaration = token
        .parent_ancestors()
        .find(|a| a.kind() == SyntaxKind::CustomTypeDecl)?;
    let variants: Vec<SyntaxNode> = declaration
        .children()
        .filter(|c| c.kind() == SyntaxKind::Variant)
        .collect();
    if variants.len() == 1 {
        return declaration_range(&declaration);
    }
    let name = token.text();
    let variant = (variants.iter()).find(|v| {
        let words = words(v);
        (words.strip_prefix(name)).is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
    });
    let Some(variant) = variant else {
        return Some(token.text_range());
    };
    let extent = |v: &SyntaxNode| variant_range(v).or_else(|| code_range(v));
    let range = extent(variant)?;
    let siblings: Vec<SyntaxElement> = (declaration.children_with_tokens())
        .filter(|e| !e.kind().is_trivia())
        .collect();
    let at = siblings.iter().position(|e| e.as_node() == Some(variant))?;
    let before = at.checked_sub(1).and_then(|i| siblings.get(i));
    let mut start = range.start();
    let mut end = range.end();
    match before.map(SyntaxElement::kind) {
        Some(SyntaxKind::Eq) => {
            start = before?.text_range().end();
            if let Some(pipe) = siblings
                .get(at + 1)
                .filter(|e| e.kind() == SyntaxKind::Pipe)
            {
                end = pipe.text_range().end();
            }
        }
        Some(SyntaxKind::Pipe) => {
            if let Some(previous) = at.checked_sub(2).and_then(|i| siblings.get(i)) {
                start = match previous {
                    SyntaxElement::Node(v) => extent(v)?.end(),
                    SyntaxElement::Token(t) => t.text_range().end(),
                };
            }
        }
        _ => {}
    }
    Some(TextRange::new(start, end))
}

/// `removeUnusedParameterCodeAction`: for each unused parameter the editor
/// sent, the parameter, its type in the signature, and the argument each
/// call passes it, across the project's files.
pub(super) fn parameter_removal_actions(fixing: &Fixing<'_>, _range: TextRange) -> Vec<Offer> {
    (fixing.request.given.iter())
        .filter(|d| d.is(Code::UnusedPattern))
        .filter_map(|d| remove_parameter(fixing, d))
        .collect()
}

/// A parameter to remove: its pattern, its place among its value's, how
/// many those are, and the value's declaration and name.
struct Parameter {
    pattern: SyntaxNode,
    index: usize,
    count: usize,
    declaration: SyntaxNode,
    name: SyntaxToken,
}

fn remove_parameter(fixing: &Fixing<'_>, diagnostic: &Diagnosed) -> Option<Offer> {
    let parameter = removable(fixing, diagnostic)?;
    let text = &*fixing.context.module.text;
    // What comes before it, comments passed over.
    let first = parameter.pattern.first_token()?;
    let previous = std::iter::successors(first.prev_token(), |t| t.prev_token())
        .find(|t| !t.kind().is_trivia())?;
    let before = previous.text_range().end();
    let mut edits = vec![remove_argument(text, before, &parameter.pattern)?];
    if let Some(signature) = child(&parameter.declaration, SyntaxKind::Signature) {
        edits.push(signature_edit(&signature, &parameter)?);
    }
    let mut changes = here(fixing, edits);
    changes.merge(call_edits(fixing, &parameter)?);
    for (_, edits) in &mut changes {
        *edits = disjoint(std::mem::take(edits))?;
    }
    let title = format!("Remove unused parameter `{}`", words(&parameter.pattern));
    Some(Offer::new(title, changes))
}

/// The parameter an unused pattern's `diagnostic` is about, where it can
/// go: not an entry point's, a package's API's or an operator's, nor a
/// recursive value's only one.
fn removable(fixing: &Fixing<'_>, diagnostic: &Diagnosed) -> Option<Parameter> {
    let context = &fixing.context;
    let root = &context.module.root();
    let token = token_at(root, diagnostic.range.start())?;
    let pattern = token
        .parent()
        .filter(|p| p.kind() == SyntaxKind::VarPattern)?;
    let definition = pattern
        .parent()
        .filter(|d| d.kind() == SyntaxKind::Definition)?;
    let declaration = definition.parent()?;
    if !matches!(
        declaration.kind(),
        SyntaxKind::ValueDecl | SyntaxKind::LetValue
    ) {
        return None;
    }
    let parameters: Vec<SyntaxNode> = (definition.children())
        .filter(|c| ast::Pattern::can_cast(c.kind()))
        .collect();
    let index = parameters.iter().position(|p| *p == pattern)?;
    let name = name_of(&definition)?;
    let top = declaration.kind() == SyntaxKind::ValueDecl;
    if top && (name.text() == "main" || exposed_by_package(context, root, name.text())) {
        // Entry points and a package's API have callers outside it.
        return None;
    }
    let operator = (root.children())
        .filter(|n| n.kind() == SyntaxKind::InfixDecl)
        .any(|n| n.last_token().is_some_and(|t| t.text() == name.text()));
    if operator {
        return None;
    }
    if parameters.len() == 1 && is_recursive(context, &declaration) {
        // Elm doesn't allow recursive values.
        return None;
    }
    Some(Parameter {
        pattern,
        index,
        count: parameters.len(),
        declaration,
        name,
    })
}

/// The parameter's type out of its value's `signature`.
fn signature_edit(signature: &SyntaxNode, parameter: &Parameter) -> Option<Edit> {
    let parts = flat_type(signature)?;
    let arrows: Vec<usize> = (parts.iter().enumerate())
        .filter(|(_, p)| p.kind() == SyntaxKind::Arrow)
        .map(|(i, _)| i)
        .collect();
    if arrows.len() < parameter.count {
        return None;
    }
    let start = match parameter.index {
        0 => parts.first()?,
        index => parts.get(arrows[index - 1] + 1)?,
    };
    let end = parts.get(arrows[parameter.index] + 1)?;
    let range = TextRange::new(start.text_range().start(), end.text_range().start());
    Some(Edit::delete(range))
}

/// The argument each call of the parameter's value passes it, out, in
/// each of the project's files; none where a file doesn't parse, or the
/// value is used other than called.
fn call_edits(fixing: &Fixing<'_>, parameter: &Parameter) -> Option<FileEdits> {
    let context = &fixing.context;
    if !context.index.modules.iter().all(|m| m.parses) {
        return None;
    }
    let (name, declared) = (parameter.name.text(), parameter.name.text_range());
    let mut changes = FileEdits::default();
    for other in &context.index.modules {
        for reference in other.root().descendants().filter(|n| {
            matches!(
                n.kind(),
                SyntaxKind::NameExpr | SyntaxKind::RecordUpdateExpr
            )
        }) {
            let Some(identifier) = last_child_token(&reference, SyntaxKind::LowerIdent) else {
                continue;
            };
            if identifier.text() != name {
                continue;
            }
            let at = identifier.text_range().start();
            // An unresolved use of the name might be this function's.
            let found =
                crate::navigation::definition(context.snapshot, context.index, &other.path, at)?;
            // Definitions answer with the declaring node.
            if found.range.start() != declared.start() || found.path != context.module.path {
                continue;
            }
            if reference.kind() != SyntaxKind::NameExpr {
                return None;
            }
            let mut target = reference.clone();
            while let Some(up) = target
                .parent()
                .filter(|p| p.kind() == SyntaxKind::ParenExpr)
            {
                target = up;
            }
            let call = target
                .parent()
                .filter(|c| c.kind() == SyntaxKind::CallExpr)?;
            let mut parts = call.children();
            if parts.next().as_ref() != Some(&target) {
                return None;
            }
            let arguments: Vec<SyntaxNode> = parts.collect();
            let argument = arguments.get(parameter.index)?;
            let before = match parameter.index {
                0 => code_range(&target)?.end(),
                index => code_range(&arguments[index - 1])?.end(),
            };
            let edit = remove_argument(&other.text, before, argument)?;
            changes.file(&other.path).push(edit);
        }
    }
    Some(changes)
}

/// `edits` in order, but those inside another, which removes them with
/// it; none where two overlap otherwise.
fn disjoint(mut edits: Vec<Edit>) -> Option<Vec<Edit>> {
    edits.sort_by(|a, b| {
        (a.range.start().cmp(&b.range.start())).then(b.range.end().cmp(&a.range.end()))
    });
    let mut kept: Vec<Edit> = Vec::new();
    for edit in edits {
        if let Some(previous) = kept.last()
            && edit.range.start() < previous.range.end()
        {
            if edit.range.end() <= previous.range.end() {
                continue;
            }
            return None;
        }
        kept.push(edit);
    }
    Some(kept)
}

/// `removeArgument`: from the end of what comes before, or from the
/// argument's start where a comment lies between.
fn remove_argument(text: &str, before: TextSize, argument: &SyntaxNode) -> Option<Edit> {
    let range = code_range(argument)?;
    let gap = &text[usize::from(before)..usize::from(range.start())];
    let start = if gap.trim().is_empty() {
        before
    } else {
        range.start()
    };
    Some(Edit::delete(TextRange::new(start, range.end())))
}

/// A signature's type as tree-sitter-elm's flat `type_expression` has its
/// children: each part and arrow of the function type, comments left out.
fn flat_type(signature: &SyntaxNode) -> Option<Vec<SyntaxElement>> {
    let mut ty = signature.children().last()?;
    let mut parts = Vec::new();
    loop {
        if ty.kind() != SyntaxKind::FunctionType {
            parts.push(SyntaxElement::Node(ty));
            return Some(parts);
        }
        let mut next = None;
        for element in ty.children_with_tokens().filter(|e| !e.kind().is_trivia()) {
            match &element {
                SyntaxElement::Node(n) if n.kind() == SyntaxKind::FunctionType => {
                    next = Some(n.clone());
                }
                _ => parts.push(element),
            }
        }
        match next {
            Some(n) => ty = n,
            None => return Some(parts),
        }
    }
}

/// Whether the module is a package's exposed one and exposes `name`.
fn exposed_by_package(context: &Context<'_>, root: &SyntaxNode, name: &str) -> bool {
    let Some((_, Outline::Pkg(outline), _)) =
        crate::project_outline(context.snapshot, context.path)
    else {
        return false;
    };
    outline
        .exposed
        .modules()
        .any(|m| m.to_string() == context.module.name)
        && crate::lens::exposes(root, name, crate::lens::Exposable::Value)
}

/// `isRecursive`: whether the value refers to itself, through the
/// module's other values.
fn is_recursive(context: &Context<'_>, declaration: &SyntaxNode) -> bool {
    let module = context.module;
    let mut pending = vec![declaration.clone()];
    let mut visited: Vec<SyntaxNode> = Vec::new();
    while let Some(current) = pending.pop() {
        if visited.contains(&current) {
            continue;
        }
        visited.push(current.clone());
        for reference in current
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::NameExpr)
        {
            let Some(identifier) = last_child_token(&reference, SyntaxKind::LowerIdent) else {
                continue;
            };
            let found = crate::navigation::definition(
                context.snapshot,
                context.index,
                context.path,
                identifier.text_range().start(),
            );
            let Some(found) = found.filter(|f| f.path == module.path) else {
                continue;
            };
            let Some(at) = module
                .root()
                .token_at_offset(found.range.start())
                .right_biased()
            else {
                continue;
            };
            // Only functions: a definition's name.
            if at
                .parent()
                .is_none_or(|p| p.kind() != SyntaxKind::Definition)
            {
                continue;
            }
            let dependency = at
                .parent_ancestors()
                .find(|a| matches!(a.kind(), SyntaxKind::ValueDecl | SyntaxKind::LetValue));
            match dependency {
                Some(d) if d == *declaration => return true,
                Some(d) => pending.push(d),
                None => {}
            }
        }
    }
    false
}
