//! Fixes for the lint rules' unused code (`removeUnusedCodeAction.ts`):
//! imports, aliases, values, constructors, type aliases and patterns,
//! and unused parameters with the arguments every call passes them
//! (`removeUnusedParameterCodeAction.ts`).

use std::collections::{BTreeMap, BTreeSet};

use super::*;

/// The lint codes `remove_unused` fixes.
pub(super) const UNUSED: &[&str] = &[
    "unused_import",
    "unused_imported_value",
    "unused_alias",
    "unused_top_level",
    "unused_pattern",
    "unused_value_constructor",
    "unused_type_alias",
];

/// For each diagnostic the editor sent, the action that removes what it
/// reports, as elm-language-server offers all of them for each one.
pub(super) fn unused_actions(fixing: &Fixing<'_>, _range: TextRange) -> Vec<Offer> {
    (fixing.given.iter())
        .filter_map(|d| {
            let (title, edits) = unused_edits(fixing, d, None);
            Some((title?, here(fixing, edits), None))
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
/// reports takes, and its title where it has one.
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
    let delete = |range: TextRange| Edit {
        range,
        text: String::new(),
    };
    match diagnostic.code.as_str() {
        "unused_import" => {
            let name = (token.parent_ancestors())
                .find(|a| a.kind() == SyntaxKind::Import)
                .and_then(|i| child(&i, SyntaxKind::ModuleName))
                .map_or_else(|| token.text().to_owned(), |n| crate::symbols::words(&n));
            let line = text[..usize::from(range.end())].matches('\n').count();
            let end = line_start(text, line + 1);
            let title = format!("Remove unused import `{name}`");
            (
                Some(title),
                vec![delete(TextRange::new(range.start(), end))],
            )
        }
        "unused_alias" => {
            let name = token_at(root, range.end())
                .filter(|t| t.kind() == SyntaxKind::UpperIdent)
                .or_else(|| root.token_at_offset(range.end()).left_biased());
            let name = name.map(|t| t.text().to_owned()).unwrap_or_default();
            let start = range.start() - TextSize::new(1);
            let title = format!("Remove unused alias `{name}`");
            (
                Some(title),
                vec![delete(TextRange::new(start, range.end()))],
            )
        }
        "unused_top_level" => match remove_function(&token) {
            Some(range) => {
                let title = format!("Remove function `{}`", token.text());
                (Some(title), vec![delete(range)])
            }
            None => (None, Vec::new()),
        },
        "unused_pattern" => {
            let field = token.parent().filter(|p| {
                p.kind() == SyntaxKind::FieldPattern
                    && p.parent()
                        .is_some_and(|r| r.kind() == SyntaxKind::RecordPattern)
            });
            let edit = match field {
                Some(field) => remove_record_field(&field).map(delete),
                None => Some(Edit {
                    range,
                    text: "_".to_owned(),
                }),
            };
            let title = format!("Fix unused pattern `{}`", token.text());
            (Some(title), edit.into_iter().collect())
        }
        "unused_value_constructor" => match remove_variant(&token) {
            Some(range) => {
                let title = format!("Remove unused value constructor `{}`", token.text());
                (Some(title), vec![delete(range)])
            }
            None => (None, Vec::new()),
        },
        "unused_type_alias" => {
            let alias = (token.parent_ancestors()).find(|a| a.kind() == SyntaxKind::TypeAliasDecl);
            match alias.and_then(|a| code_range(&a)) {
                Some(range) => {
                    let title = format!("Remove unused type alias `{}`", token.text());
                    (Some(title), vec![delete(range)])
                }
                None => (None, Vec::new()),
            }
        }
        "unused_imported_value" => imported_value_edits(fixing, &token, removed),
        _ => (None, Vec::new()),
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
    let Some(import) = token
        .parent_ancestors()
        .find(|a| a.kind() == SyntaxKind::Import)
    else {
        return (None, Vec::new());
    };
    let Some(module) = child(&import, SyntaxKind::ModuleName).map(|n| crate::symbols::words(&n))
    else {
        return (None, Vec::new());
    };
    let list = child(&import, SyntaxKind::ExposingList);
    let exposed: Vec<SyntaxNode> = (list.iter())
        .flat_map(|l| l.children())
        .filter(|n| matches!(n.kind(), SyntaxKind::ExposedValue | SyntaxKind::ExposedType))
        .collect();
    let all: Vec<&str> = exposed.iter().map(|n| written(text, n)).collect();
    let name = token.text();
    let fix_all = removed.is_some();
    let mut gone = BTreeSet::new();
    if let Some(removed) = removed {
        let set = removed.entry(module.clone()).or_default();
        set.insert(name.to_owned());
        gone = set.clone();
        // Every value of the list goes: the list goes.
        if all.iter().all(|v| gone.contains(*v))
            && let Some(edit) = list.as_ref().and_then(|l| without_list(text, l))
        {
            return (None, vec![edit]);
        }
    }
    let Some(edit) = remove_from_import(text, list.as_ref(), &exposed, name, false) else {
        return (None, Vec::new());
    };
    // The values a fix-all removes from a list's end: the comma before the
    // first of them goes too.
    if fix_all && all.len() > 2 {
        let reversed: Vec<&str> = all.iter().rev().copied().collect();
        if let Some(first_used) = reversed.iter().position(|v| !gone.contains(*v))
            && first_used >= 2
            && let Some(with_comma) = remove_from_import(
                text,
                list.as_ref(),
                &exposed,
                reversed[first_used - 1],
                true,
            )
        {
            return (None, vec![with_comma, edit]);
        }
    }
    // elm-language-server says "type" only where its node is the exposed
    // type itself, and the diagnostic starts at the name.
    (Some(format!("Remove unused value `{name}`")), vec![edit])
}

/// `removeValueFromImport`: the whole exposing list when `name` is all
/// it exposes, else `name` out of it.
pub(super) fn remove_from_import(
    text: &str,
    list: Option<&SyntaxNode>,
    exposed: &[SyntaxNode],
    name: &str,
    force: bool,
) -> Option<Edit> {
    if exposed.len() == 1 && written(text, &exposed[0]) == name {
        return without_list(text, list?);
    }
    remove_exposed(exposed, text, name, force)
}

/// `removeImportExposingList`: from the space before `exposing` to the
/// list's end.
fn without_list(_text: &str, list: &SyntaxNode) -> Option<Edit> {
    let range = code_range(list)?;
    Some(Edit {
        range: TextRange::new(range.start() - TextSize::new(1), range.end()),
        text: String::new(),
    })
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

/// `removeRecordPatternValue`: a field of a record pattern, with the
/// comma before it, or else the one after it.
fn remove_record_field(field: &SyntaxNode) -> Option<TextRange> {
    let range = code_range(field)?;
    let siblings: Vec<SyntaxElement> = (field.parent()?.children_with_tokens())
        .filter(|e| !matches!(e.kind(), SyntaxKind::Whitespace | SyntaxKind::Newline))
        .collect();
    let at = siblings.iter().position(|e| e.as_node() == Some(field))?;
    let comma_before = at
        .checked_sub(1)
        .and_then(|i| siblings.get(i))
        .filter(|e| e.kind() == SyntaxKind::Comma);
    let mut start = range.start();
    let mut end = range.end();
    if let Some(comma) = comma_before {
        start = comma.text_range().start();
    } else if siblings
        .get(at + 1)
        .is_some_and(|e| e.kind() == SyntaxKind::Comma)
        && let Some(after) = siblings.get(at + 2)
    {
        end = after.text_range().start();
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
        return crate::symbols::declaration_range(&declaration);
    }
    let name = token.text();
    let variant = (variants.iter()).find(|v| {
        let words = crate::symbols::words(v);
        words == name || words.starts_with(&format!("{name} "))
    });
    let Some(variant) = variant else {
        return Some(token.text_range());
    };
    let extent = |v: &SyntaxNode| crate::symbols::variant_range(v).or_else(|| code_range(v));
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
    (fixing.given.iter())
        .filter(|d| d.code == "unused_pattern")
        .filter_map(|d| remove_parameter(fixing, d))
        .collect()
}

fn remove_parameter(fixing: &Fixing<'_>, diagnostic: &Diagnosed) -> Option<Offer> {
    let context = &fixing.context;
    let module = context.module;
    let (root, text) = (&module.root(), &*module.text);
    let token = token_at(root, diagnostic.range.start())?;
    let parameter = token
        .parent()
        .filter(|p| p.kind() == SyntaxKind::VarPattern)?;
    let definition = parameter
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
        .filter(|c| crate::lint::is_pattern(c.kind()))
        .collect();
    let index = parameters.iter().position(|p| *p == parameter)?;
    let name = name_of(&definition)?;
    let top = declaration.kind() == SyntaxKind::ValueDecl;
    if top && (name.text() == "main" || exposed_by_package(context, root, text, name.text())) {
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
    // What comes before it, comments passed over.
    let previous = std::iter::successors(parameter.first_token()?.prev_token(), |t| t.prev_token())
        .find(|t| !t.kind().is_trivia())?;
    let path = context.path.to_owned();
    let mut changes: FileEdits = vec![(
        path.clone(),
        vec![remove_argument(
            text,
            previous.text_range().end(),
            &parameter,
        )?],
    )];
    if let Some(signature) = child(&declaration, SyntaxKind::Signature) {
        let parts = flat_type(&signature)?;
        let arrows: Vec<usize> = (parts.iter().enumerate())
            .filter(|(_, p)| p.kind() == SyntaxKind::Arrow)
            .map(|(i, _)| i)
            .collect();
        if arrows.len() < parameters.len() {
            return None;
        }
        let start = if index == 0 {
            parts.first()?
        } else {
            parts.get(arrows[index - 1] + 1)?
        };
        let end = parts.get(arrows[index] + 1)?;
        changes[0].1.push(Edit {
            range: TextRange::new(start.text_range().start(), end.text_range().start()),
            text: String::new(),
        });
    }
    // Each call, in each of the project's files.
    let declared = name.text_range();
    for other in &context.index.modules {
        if !nomi_syntax::parse_module(&other.text).errors().is_empty() {
            return None;
        }
        for reference in other.root().descendants().filter(|n| {
            matches!(
                n.kind(),
                SyntaxKind::NameExpr | SyntaxKind::RecordUpdateExpr
            )
        }) {
            let Some(identifier) = (reference.children_with_tokens())
                .filter_map(|e| e.into_token())
                .filter(|t| t.kind() == SyntaxKind::LowerIdent)
                .last()
            else {
                continue;
            };
            if identifier.text() != name.text() {
                continue;
            }
            let found = crate::navigation::definition(
                context.snapshot,
                context.index,
                &other.path,
                identifier.text_range().start(),
            );
            let Some(found) = found else {
                // An unresolved use of the name might be this function's.
                return None;
            };
            // Definitions answer with the declaring node.
            if found.range.start() != declared.start() || found.path != module.path {
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
            let argument = arguments.get(index)?;
            let before = match index {
                0 => code_range(&target)?.end(),
                _ => code_range(&arguments[index - 1])?.end(),
            };
            let edit = remove_argument(&other.text, before, argument)?;
            match changes.iter_mut().find(|(p, _)| *p == other.path) {
                Some((_, edits)) => edits.push(edit),
                None => changes.push((other.path.clone(), vec![edit])),
            }
        }
    }
    // A call inside an argument that goes: the outer deletion keeps it.
    for (_, edits) in &mut changes {
        edits.sort_by(|a, b| {
            a.range
                .start()
                .cmp(&b.range.start())
                .then(b.range.end().cmp(&a.range.end()))
        });
        let mut disjoint: Vec<Edit> = Vec::new();
        for edit in edits.drain(..) {
            if let Some(previous) = disjoint.last()
                && edit.range.start() < previous.range.end()
            {
                if edit.range.end() <= previous.range.end() {
                    continue;
                }
                return None;
            }
            disjoint.push(edit);
        }
        *edits = disjoint;
    }
    let title = format!(
        "Remove unused parameter `{}`",
        crate::symbols::words(&parameter)
    );
    Some((title, changes, None))
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
    Some(Edit {
        range: TextRange::new(start, range.end()),
        text: String::new(),
    })
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
                    next = Some(n.clone())
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
fn exposed_by_package(context: &Context<'_>, root: &SyntaxNode, text: &str, name: &str) -> bool {
    let Some(manifest) = nomi_engine::index::manifest(context.snapshot, context.path) else {
        return false;
    };
    let Ok(bytes) = context.snapshot.read(&manifest) else {
        return false;
    };
    let Ok((Outline::Pkg(outline), _)) = Outline::detect(bytes.as_bytes()) else {
        return false;
    };
    outline
        .exposed
        .modules()
        .any(|m| m.to_string() == context.module.name)
        && crate::lens::exposes(root, text, name, true)
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
            let Some(identifier) = (reference.children_with_tokens())
                .filter_map(|e| e.into_token())
                .filter(|t| t.kind() == SyntaxKind::LowerIdent)
                .last()
            else {
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
            if !at
                .parent()
                .is_some_and(|p| p.kind() == SyntaxKind::Definition)
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
