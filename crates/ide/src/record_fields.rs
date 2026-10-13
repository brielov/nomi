//! Record fields to complete (plan 9.8): after a record's dot, or where a
//! record, an update or a record pattern names its fields. As
//! elm-language-server, the record's alias is found from the syntax
//! first: the annotation of the parameter that binds it, of the function
//! that returns it, or of the field it's the value of. Inference answers
//! where that finds none, as far as the module checks.
//!
//! Each field comes as a preselected field, its type after it, and its
//! documentation naming the alias.

use nomi_base::{TextRange, TextSize};
use nomi_compiler::types::{self, error};
use nomi_engine::Snapshot;
use nomi_engine::index::{Index, Indexed};
use nomi_syntax::{SyntaxKind, SyntaxNode};

use crate::Edit;
use crate::completion::{Completion, CompletionKind};
use crate::symbols::{child, code_range, token};

/// The fields of the record before the dot at `dot`.
pub(crate) fn after(
    snapshot: &Snapshot,
    index: &Index,
    module: &Indexed,
    dot: TextSize,
    range: TextRange,
) -> Vec<Completion> {
    let root = &module.root;
    let text = &*module.text;
    let at = usize::from(dot);
    let name_start = text[..at]
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
        .last()
        .map_or(at, |(i, _)| i);
    let name = &text[name_start..at];
    if !name.is_empty() && name.starts_with(|c: char| c.is_lowercase()) {
        let fields = alias_of_name(index, module, TextSize::new(name_start as u32), name)
            .map(|alias| alias_fields(&alias, range));
        if let Some(fields) = fields.filter(|f| !f.is_empty()) {
            return fields;
        }
    }
    let _ = snapshot;
    // The innermost expression before the dot, which the access binds
    // tighter than anything around it.
    let before_dot = |root: &SyntaxNode| {
        let before = root.token_at_offset(dot).left_biased()?;
        before
            .parent_ancestors()
            .take_while(|a| a.text_range().end() == dot)
            .find(|a| crate::completion::is_expr(a.kind()))
    };
    // A module that parses and compiles as written types it best; where
    // it doesn't parse, elm-language-server's tree is broken there too.
    let parsed = !root
        .descendants_with_tokens()
        .any(|e| matches!(e.kind(), SyntaxKind::Error | SyntaxKind::ErrorToken));
    if parsed
        && let Some(canonical) = &module.canonical
        && let Some(record) = before_dot(root)
    {
        let (_, types) = types::infer_types(index.compat, canonical);
        let fields = typed_fields(index, module, root, &types, &record, range);
        if !fields.is_empty() {
            return fields;
        }
    }
    // Else as far as inference reaches without the dot and what follows
    // it, the record typed by its context alone. The field's whole name
    // goes, the part after the cursor too, so the record's name doesn't
    // run into it.
    let rest = text[usize::from(range.end())..]
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(text.len() - usize::from(range.end()));
    let cut = TextRange::new(dot, range.end() + TextSize::new(rest as u32));
    let Some((patched, types)) = patched(index, module, cut, "") else {
        return Vec::new();
    };
    let Some(record) = before_dot(&patched) else {
        return Vec::new();
    };
    typed_fields(index, module, &patched, &types, &record, range)
}

/// The fields of `record`'s type, in `types` of the tree `root`: its
/// own, or, for a variable, its binding's.
fn typed_fields(
    index: &Index,
    module: &Indexed,
    root: &SyntaxNode,
    types: &types::ExprTypes,
    record: &SyntaxNode,
    range: TextRange,
) -> Vec<Completion> {
    let Some(at) = code_range(record) else {
        return Vec::new();
    };
    let mut ranges = vec![at];
    if record.kind() == SyntaxKind::NameExpr {
        ranges.extend(crate::scopes::local(root, at.start()).map(|(b, _)| b));
    }
    let typed: Vec<&error::Type> = (types.iter())
        .filter(|(r, _)| crate::hover::trimmed(root, *r).is_some_and(|r| ranges.contains(&r)))
        .map(|(_, ty)| ty)
        .collect();
    let ty = typed
        .iter()
        .find(|t| matches!(t, error::Type::Alias(..)))
        .or_else(|| typed.iter().find(|t| matches!(t, error::Type::Record(..))));
    let Some(ty) = ty else {
        return Vec::new();
    };
    // Its fields' types are written with every name bare.
    let names = crate::hover::Names::bare();
    let (alias, record) = match ty {
        error::Type::Alias(_, name, _, body) => (name.as_str(), &**body),
        ty => ("", *ty),
    };
    // An alias's fields as it declares them.
    if !alias.is_empty()
        && let Some(declared) = named_alias_by_name(index, module, alias)
    {
        return alias_fields(&declared, range);
    }
    let error::Type::Record(fields, _) = record else {
        return Vec::new();
    };
    (fields.iter())
        .map(|(name, ty)| {
            let ty = crate::hover::render_error(&names, ty);
            field_completion(name, &ty, alias, range)
        })
        .collect()
}

/// The tree and types of `module` with `insert` in place of `replaced`, where
/// that parses and canonicalizes: a half-written record made whole.
pub(crate) fn patched(
    index: &Index,
    module: &Indexed,
    replaced: TextRange,
    insert: &str,
) -> Option<(SyntaxNode, types::ExprTypes)> {
    patched_all(index, module, &[(replaced, insert.to_owned())])
}

/// [`patched`] with each of `edits`, in order and apart, made at once.
pub(crate) fn patched_all(
    index: &Index,
    module: &Indexed,
    edits: &[(TextRange, String)],
) -> Option<(SyntaxNode, types::ExprTypes)> {
    use nomi_syntax::ast::{self, AstNode};
    let mut text = module.text.to_string();
    for (replaced, insert) in edits.iter().rev() {
        text.replace_range(
            usize::from(replaced.start())..usize::from(replaced.end()),
            insert,
        );
    }
    let parse = nomi_syntax::parse_module(&text);
    if !parse.errors().is_empty() {
        return None;
    }
    let syntax = ast::Module::cast(parse.syntax())?;
    let hir = nomi_compiler::hir::lower(&syntax);
    let (canonical, _) = nomi_compiler::canonicalize::module::canonicalize_with_references(
        index.compat,
        &index.package,
        &index.interfaces,
        &hir,
    );
    let (_, types) = types::infer_types(index.compat, &canonical.ok()?);
    Some((parse.syntax(), types))
}

/// The fields to complete at `offset` where a record, an update or a
/// record pattern names them; none where `offset` isn't such a place.
pub(crate) fn inside(
    snapshot: &Snapshot,
    index: &Index,
    module: &Indexed,
    offset: TextSize,
    range: TextRange,
) -> Option<Vec<Completion>> {
    let text = &*module.text;
    let at = usize::from(range.start());
    let open = open_brace(text, at)?;
    // A record type's field is named, not completed: the general list.
    let brace = (module.root.token_at_offset(TextSize::new(open as u32))).right_biased();
    if brace.and_then(|t| t.parent()).is_some_and(|p| {
        matches!(
            p.kind(),
            SyntaxKind::RecordType | SyntaxKind::RecordExtension
        )
    }) {
        return None;
    }
    let between = &text[open + 1..at];
    // A field's name goes after `{`, `,` or `|`, alone.
    let last = between.rfind([',', '|']).map_or(0, |i| i + 1);
    if !between[last..].trim().is_empty() {
        return None;
    }
    let before = text[..open].trim_end();
    let base = between
        .split('|')
        .next()
        .filter(|_| between.contains('|'))
        .map(str::trim)
        .filter(|b| !b.is_empty() && b.chars().all(|c| c.is_alphanumeric() || c == '_'));
    let alias = match base {
        // An update: its record's.
        Some(base) => {
            let at = open + 1 + between.find(base)?;
            alias_of_name(index, module, TextSize::new(at as u32), base)
        }
        None if in_pattern(text, open) => {
            // A record pattern: its parameter's annotation.
            return Some(pattern_fields(
                index,
                module,
                TextSize::new(open as u32),
                range,
            ));
        }
        // A record: the field it's the value of, or what the function
        // returns.
        None => match field_before(before).filter(|_| open_brace(text, open).is_some()) {
            Some(field) => outer_field_alias(index, module, text, open, field),
            None => returned_alias(index, module, TextSize::new(open as u32)),
        },
    };
    let _ = snapshot;
    if let Some(alias) = alias {
        return Some(alias_fields(&alias, range));
    }
    // As far as inference reaches: the record made whole.
    let word = &text[at..usize::from(offset)];
    let insert = if word.is_empty() {
        "nomiField = Debug.todo \"\""
    } else {
        " = Debug.todo \"\""
    };
    let found =
        patched(index, module, TextRange::empty(offset), insert).and_then(|(root, types)| {
            let record = root
                .token_at_offset(TextSize::new(open as u32))
                .right_biased()?
                .parent()
                .filter(|p| {
                    matches!(
                        p.kind(),
                        SyntaxKind::RecordExpr | SyntaxKind::RecordUpdateExpr
                    )
                })?;
            Some(typed_fields(index, module, &root, &types, &record, range))
        });
    Some(found.unwrap_or_default())
}

/// A type alias's declaration, in the file that has it.
struct Alias {
    decl: SyntaxNode,
}

/// The fields `alias` declares, as completions.
fn alias_fields(alias: &Alias, range: TextRange) -> Vec<Completion> {
    let name = token(&alias.decl, SyntaxKind::UpperIdent).unwrap_or_default();
    let Some(record) = alias
        .decl
        .children()
        .find(|c| c.kind() == SyntaxKind::RecordType)
    else {
        return Vec::new();
    };
    (record.children())
        .filter(|c| c.kind() == SyntaxKind::FieldType)
        .filter_map(|field| {
            let label = token(&field, SyntaxKind::LowerIdent)?;
            let ty = field.children().find(|c| crate::lint::is_type(c.kind()))?;
            let ty = text_of(&ty)?;
            Some(field_completion(&label, &ty, &name, range))
        })
        .collect()
}

/// `createFieldOrParameterCompletion` with `createHintForTypeAliasReference`.
pub(crate) fn field_completion(label: &str, ty: &str, alias: &str, range: TextRange) -> Completion {
    let refers = if alias.is_empty() {
        format!("Refers to the `{label}` field")
    } else {
        format!("Refers to the `{label}` field on `{alias}`")
    };
    Completion {
        label: label.to_owned(),
        kind: CompletionKind::Field,
        sort_text: None,
        filter_text: None,
        documentation: Some(format!("\n```elm\n{ty}\n```\n\n\n---\n\n{refers}")),
        detail: None,
        label_detail: (ty != "unknown").then(|| format!(" : {ty}")),
        label_description: None,
        preselect: true,
        edit: Some(Edit {
            range,
            text: label.to_owned(),
        }),
        snippet: None,
        additional_edits: Vec::new(),
    }
}

/// The declaration of the alias of the variable `name` written at `at`.
pub(crate) fn alias_declaration(
    index: &Index,
    module: &Indexed,
    at: TextSize,
    name: &str,
) -> Option<SyntaxNode> {
    alias_of_name(index, module, at, name).map(|alias| alias.decl)
}

/// The alias of the variable `name` written at `at`: by the annotation
/// of the parameter binding it, the field a record pattern binds it as,
/// or the function it names.
fn alias_of_name(index: &Index, module: &Indexed, at: TextSize, name: &str) -> Option<Alias> {
    let root = &module.root;
    if let Some((binder, _)) = crate::scopes::local(root, at) {
        let token = root.token_at_offset(binder.start()).right_biased()?;
        let parent = token.parent()?;
        return match parent.kind() {
            // A record pattern's field: that field's type in the pattern's.
            SyntaxKind::FieldPattern => {
                let pattern = parent.parent()?;
                let outer = pattern_alias(index, module, &pattern)?;
                field_alias(index, module, &outer, name)
            }
            _ => {
                let pattern = parent
                    .ancestors()
                    .take_while(|a| crate::lint::is_pattern(a.kind()))
                    .last()?;
                param_alias(index, module, &pattern)
            }
        };
    }
    // A top-level value: what it is.
    let decl = (root.children())
        .filter(|n| n.kind() == SyntaxKind::ValueDecl)
        .find(|d| {
            child(d, SyntaxKind::Definition)
                .and_then(|def| crate::lint::first_token(&def, SyntaxKind::LowerIdent))
                .is_some_and(|t| t.text() == name)
        })?;
    let signature = child(&decl, SyntaxKind::Signature)?;
    let ty = signature
        .children()
        .find(|c| crate::lint::is_type(c.kind()))?;
    named_alias(index, module, &result_type(&ty))
}

/// The alias a parameter's pattern is annotated as.
fn param_alias(index: &Index, module: &Indexed, pattern: &SyntaxNode) -> Option<Alias> {
    let definition = pattern
        .parent()
        .filter(|p| p.kind() == SyntaxKind::Definition)?;
    let position = (definition.children())
        .filter(|c| crate::lint::is_pattern(c.kind()))
        .position(|c| c == *pattern)?;
    let signature = definition
        .prev_sibling()
        .filter(|s| s.kind() == SyntaxKind::Signature)?;
    let ty = signature
        .children()
        .find(|c| crate::lint::is_type(c.kind()))?;
    let argument = arguments(&ty).into_iter().nth(position)?;
    named_alias(index, module, &argument)
}

/// The alias a record pattern is annotated as, as a parameter.
fn pattern_alias(index: &Index, module: &Indexed, pattern: &SyntaxNode) -> Option<Alias> {
    let outermost = pattern
        .ancestors()
        .take_while(|a| crate::lint::is_pattern(a.kind()))
        .last()?;
    param_alias(index, module, &outermost)
}

/// The fields of the record pattern whose `{` is at `open`.
fn pattern_fields(
    index: &Index,
    module: &Indexed,
    open: TextSize,
    range: TextRange,
) -> Vec<Completion> {
    let root = &module.root;
    let pattern = root
        .token_at_offset(open)
        .right_biased()
        .and_then(|t| t.parent())
        .filter(|p| p.kind() == SyntaxKind::RecordPattern);
    if let Some(alias) = pattern
        .as_ref()
        .and_then(|p| pattern_alias(index, module, p))
    {
        return alias_fields(&alias, range);
    }
    // As far as inference reaches: the pattern with a field in it.
    let at = usize::from(range.end());
    let insert = if range.is_empty() { "nomiField" } else { "" };
    let found = patched(
        index,
        module,
        TextRange::empty(TextSize::new(at as u32)),
        insert,
    )
    .and_then(|(root, types)| {
        let pattern = root
            .token_at_offset(open)
            .right_biased()?
            .parent()
            .filter(|p| p.kind() == SyntaxKind::RecordPattern)?;
        Some(typed_fields(index, module, &root, &types, &pattern, range))
    });
    found.unwrap_or_default()
}

/// The alias the field `field` of `alias` is.
fn field_alias(index: &Index, module: &Indexed, alias: &Alias, field: &str) -> Option<Alias> {
    let record = alias
        .decl
        .children()
        .find(|c| c.kind() == SyntaxKind::RecordType)?;
    let declared = (record.children())
        .filter(|c| c.kind() == SyntaxKind::FieldType)
        .find(|f| token(f, SyntaxKind::LowerIdent).as_deref() == Some(field))?;
    let ty = declared
        .children()
        .find(|c| crate::lint::is_type(c.kind()))?;
    named_alias(index, module, &ty)
}

/// The alias of the record that is the value of `field` in the record
/// whose `{` comes before `open`.
fn outer_field_alias(
    index: &Index,
    module: &Indexed,
    text: &str,
    open: usize,
    field: &str,
) -> Option<Alias> {
    let outer = open_brace(text, open)?;
    let between = &text[outer + 1..open];
    let base = between
        .split('|')
        .next()
        .filter(|_| between.contains('|'))
        .map(str::trim);
    let outer_alias = match base {
        Some(base) => {
            let at = outer + 1 + between.find(base)?;
            alias_of_name(index, module, TextSize::new(at as u32), base)?
        }
        None => returned_alias(index, module, TextSize::new(outer as u32))?,
    };
    field_alias(index, module, &outer_alias, field)
}

/// The alias the function around `at` returns.
fn returned_alias(index: &Index, module: &Indexed, at: TextSize) -> Option<Alias> {
    let token = module.root.token_at_offset(at).right_biased()?;
    let decl = token
        .parent_ancestors()
        .find(|a| matches!(a.kind(), SyntaxKind::ValueDecl | SyntaxKind::LetValue))?;
    let signature = child(&decl, SyntaxKind::Signature)?;
    let ty = signature
        .children()
        .find(|c| crate::lint::is_type(c.kind()))?;
    named_alias(index, module, &result_type(&ty))
}

/// The declaration of the alias `name`: the module's own, or another
/// module's of the project.
fn named_alias_by_name(index: &Index, module: &Indexed, name: &str) -> Option<Alias> {
    let declares = |root: &SyntaxNode| {
        (root.children())
            .filter(|n| n.kind() == SyntaxKind::TypeAliasDecl)
            .find(|d| token(d, SyntaxKind::UpperIdent).as_deref() == Some(name))
    };
    if let Some(decl) = declares(&module.root) {
        return Some(Alias { decl });
    }
    index
        .modules
        .iter()
        .find_map(|m| declares(&m.root))
        .map(|decl| Alias { decl })
}

/// The alias a type names: its first capitalized name's.
fn named_alias(index: &Index, module: &Indexed, ty: &SyntaxNode) -> Option<Alias> {
    let named = std::iter::once(ty.clone())
        .chain(ty.descendants())
        .find(|n| n.kind() == SyntaxKind::NamedType)?;
    let name = token(&named, SyntaxKind::UpperIdent)?;
    let declares = |root: &SyntaxNode| {
        (root.children())
            .filter(|n| n.kind() == SyntaxKind::TypeAliasDecl)
            .find(|d| token(d, SyntaxKind::UpperIdent).as_deref() == Some(name.as_str()))
    };
    if let Some(decl) = declares(&module.root) {
        return Some(Alias { decl });
    }
    index
        .modules
        .iter()
        .find_map(|m| declares(&m.root))
        .map(|decl| Alias { decl })
}

/// A function type's arguments, in order.
fn arguments(ty: &SyntaxNode) -> Vec<SyntaxNode> {
    let mut found = Vec::new();
    let mut ty = ty.clone();
    while ty.kind() == SyntaxKind::FunctionType {
        let mut parts = ty.children().filter(|c| crate::lint::is_type(c.kind()));
        let (Some(argument), Some(result)) = (parts.next(), parts.next()) else {
            break;
        };
        found.push(argument);
        ty = result;
    }
    found
}

/// A function type's result.
fn result_type(ty: &SyntaxNode) -> SyntaxNode {
    let mut ty = ty.clone();
    while ty.kind() == SyntaxKind::FunctionType {
        let Some(result) = ty
            .children()
            .filter(|c| crate::lint::is_type(c.kind()))
            .nth(1)
        else {
            break;
        };
        ty = result;
    }
    ty
}

/// The `{` around `at` that isn't closed before it.
fn open_brace(text: &str, at: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in text[..at].char_indices().rev() {
        match c {
            '}' => depth += 1,
            '{' if depth == 0 => return Some(i),
            '{' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// The field a record is the value of: `field = {`.
fn field_before(before: &str) -> Option<&str> {
    let before = before.strip_suffix('=')?.trim_end();
    let start = before
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
        .last()?
        .0;
    let field = &before[start..];
    field
        .starts_with(|c: char| c.is_lowercase())
        .then_some(field)
}

/// Whether the `{` at `open` starts a pattern: a parameter's, or a case
/// branch's, before its `=` or `->`.
fn in_pattern(text: &str, open: usize) -> bool {
    let rest = &text[open..];
    let end = rest.find('\n').unwrap_or(rest.len());
    let line_rest = &rest[..end];
    if line_rest.contains("->") {
        return true;
    }
    let line_start = text[..open].rfind('\n').map_or(0, |i| i + 1);
    let head = &text[line_start..open];
    // `f {` on a declaration's line, before its `=`.
    let declares = head.trim_start().starts_with(|c: char| c.is_lowercase()) && !head.contains('=');
    declares && line_rest.contains('=')
}

fn text_of(node: &SyntaxNode) -> Option<String> {
    let range = code_range(node)?;
    let start = range.start() - node.text_range().start();
    Some(node.text().slice(start..start + range.len()).to_string())
}
