//! Record fields by type (plan 9.6b): a field written in an access, an
//! accessor, a record, an update or a pattern is declared where the
//! record's type writes it. The record's inferred type keeps the alias
//! it's written as, and the field is that alias's, or one of its
//! arguments'. A variable a record pattern binds is its field's, as
//! elm-language-server has it. A record whose type no alias names has
//! nowhere to point to.

use nomi_base::TextSize;
use nomi_compiler::types::{self, error};
use nomi_engine::Snapshot;
use nomi_engine::index::{Index, Indexed};
use nomi_syntax::{SyntaxKind, SyntaxNode, parse_module};

use crate::navigation::{Location, module_path};
use crate::symbols::{code_range, token};

/// Where the field at `offset` in `module` is declared.
pub(crate) fn definition(
    snapshot: &Snapshot,
    index: &Index,
    module: &Indexed,
    offset: TextSize,
) -> Option<Location> {
    // A field's declaration is its own.
    if let Some(declared) = declaration_at(module, offset) {
        return Some(declared);
    }
    written(&module.root, offset)?;
    let canonical = module.canonical.as_ref()?;
    let (_, types) = types::infer_types(index.compat, canonical);
    resolve(snapshot, index, module, &types, offset)
}

/// Every place the project writes the field at `offset` in `module`,
/// declared where it is: its declaration where `with_declaration`, then
/// each access, accessor, record, update, pattern and variable a pattern
/// binds; none where the offset names no field.
pub(crate) fn references(
    snapshot: &Snapshot,
    index: &Index,
    module: &Indexed,
    offset: TextSize,
    with_declaration: bool,
) -> Option<Vec<Location>> {
    let declared = match declaration_at(module, offset) {
        Some(declared) => declared,
        None => definition(snapshot, index, module, offset)?,
    };
    let text = snapshot.read(&declared.path).ok()?;
    let root = parse_module(&text).syntax();
    let declaration = root.covering_element(declared.range).into_node()?;
    let declaration = declaration
        .ancestors()
        .find(|n| n.kind() == SyntaxKind::FieldType)?;
    let name = crate::lint::first_token(&declaration, SyntaxKind::LowerIdent)?;
    let mut found = Vec::new();
    if with_declaration {
        found.push(Location {
            path: declared.path.clone(),
            range: name.text_range(),
        });
    }
    for other in &index.modules {
        let uses: Vec<_> = (other.root.descendants_with_tokens())
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == SyntaxKind::LowerIdent && t.text() == name.text())
            .filter(|t| written(&other.root, t.text_range().start()).is_some())
            .collect();
        if uses.is_empty() {
            continue;
        }
        let Some(canonical) = &other.canonical else {
            continue;
        };
        let (_, types) = types::infer_types(index.compat, canonical);
        for token in uses {
            let at = token.text_range().start();
            if resolve(snapshot, index, other, &types, at).as_ref() == Some(&declared) {
                found.push(Location {
                    path: other.path.clone(),
                    range: token.text_range(),
                });
            }
        }
    }
    Some(found)
}

/// The field declaration whose name is at `offset`, in `module`'s own
/// types.
fn declaration_at(module: &Indexed, offset: TextSize) -> Option<Location> {
    let token =
        (module.root.token_at_offset(offset)).find(|t| t.kind() == SyntaxKind::LowerIdent)?;
    let field = token
        .parent()
        .filter(|p| p.kind() == SyntaxKind::FieldType)?;
    Some(Location {
        path: module.path.clone(),
        range: code_range(&field)?,
    })
}

/// Where the field at `offset` is declared, given `module`'s types.
fn resolve(
    snapshot: &Snapshot,
    index: &Index,
    module: &Indexed,
    types: &types::ExprTypes,
    offset: TextSize,
) -> Option<Location> {
    let (record, field) = written(&module.root, offset)?;
    let range = code_range(&record)?;
    // A node is typed as what its context expects; a variable's alias is
    // its binding's, typed from the annotation.
    let mut ranges = vec![range];
    if record.kind() == SyntaxKind::NameExpr {
        let bound = (module.references.iter())
            .find(|r| r.range.contains_range(range) || range.contains_range(r.range))
            .and_then(|r| match r.target {
                nomi_compiler::canonicalize::references::Target::Local(binder) => Some(binder),
                _ => None,
            })
            .or_else(|| crate::scopes::local(&module.root, range.start()).map(|(b, _)| b));
        ranges.extend(bound);
    }
    let typed: Vec<&error::Type> = (types.iter())
        .filter(|(r, _)| {
            let r = crate::hover::trimmed(&module.root, *r);
            r.is_some_and(|r| ranges.contains(&r))
        })
        .map(|(_, ty)| ty)
        .collect();
    let aliased = |ty: &&&error::Type| match ty {
        error::Type::Alias(..) => true,
        error::Type::Lambda(arg, _, _) => matches!(**arg, error::Type::Alias(..)),
        _ => false,
    };
    let ty = typed.iter().find(aliased).or(typed.last()).copied()?;
    let ty = match record.kind() {
        // An accessor is a function of the record.
        SyntaxKind::AccessorExpr => match ty {
            error::Type::Lambda(arg, _, _) => &**arg,
            _ => return None,
        },
        _ => ty,
    };
    declared(snapshot, index, ty, &field).or_else(|| structural(index, ty, &field))
}

/// Where a record type no alias names declares `field`: the one record
/// type in the project's declarations with the same fields, if only one
/// has them. Unification keeps an alias on one side only, so a record
/// built in a list or returned from a lambda may have lost its name.
fn structural(index: &Index, ty: &error::Type, field: &str) -> Option<Location> {
    let error::Type::Record(fields, extension) = ty else {
        return None;
    };
    let wanted: Vec<&str> = fields.keys().map(String::as_str).collect();
    let closed = matches!(extension, error::Extension::Closed);
    let mut found = Vec::new();
    for module in &index.modules {
        for record in module
            .root
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::RecordType)
        {
            let declared: Vec<(String, SyntaxNode)> = (record.children())
                .filter(|c| c.kind() == SyntaxKind::FieldType)
                .filter_map(|f| Some((token(&f, SyntaxKind::LowerIdent)?, f)))
                .collect();
            let names: Vec<&str> = declared.iter().map(|(n, _)| n.as_str()).collect();
            let fits = if closed {
                names.len() == wanted.len() && wanted.iter().all(|w| names.contains(w))
            } else {
                wanted.iter().all(|w| names.contains(w))
            };
            if let Some((_, node)) = declared.iter().find(|(n, _)| n == field).filter(|_| fits) {
                found.push(Location {
                    path: module.path.clone(),
                    range: code_range(node)?,
                });
            }
        }
    }
    found.dedup();
    match found.as_slice() {
        [only] => Some(only.clone()),
        _ => None,
    }
}

/// The record node whose type declares the field at `offset`, and the
/// field's name.
fn written(root: &SyntaxNode, offset: TextSize) -> Option<(SyntaxNode, String)> {
    let token = (root.token_at_offset(offset)).find(|t| t.kind() == SyntaxKind::LowerIdent)?;
    let parent = token.parent()?;
    let name = token.text().to_owned();
    let record = match parent.kind() {
        SyntaxKind::FieldAccessExpr => {
            // The name after the dot, not the record before it.
            let first = parent.children().next()?;
            if first.text_range().contains_range(token.text_range()) {
                return None;
            }
            first
        }
        SyntaxKind::AccessorExpr => parent,
        SyntaxKind::Field => parent.parent()?,
        SyntaxKind::RecordUpdateExpr => return None,
        SyntaxKind::FieldPattern => parent.parent()?,
        // A variable a record pattern binds.
        SyntaxKind::NameExpr => {
            let (binder, _) = crate::scopes::local(root, offset)?;
            let bound = root.token_at_offset(binder.start()).right_biased()?;
            let pattern = bound
                .parent()
                .filter(|p| p.kind() == SyntaxKind::FieldPattern)?;
            pattern.parent()?
        }
        _ => return None,
    };
    Some((record, name))
}

/// Where `ty`, a record's type, declares `field`: the alias it's written
/// as, or an argument of it.
fn declared(snapshot: &Snapshot, index: &Index, ty: &error::Type, field: &str) -> Option<Location> {
    let error::Type::Alias(home, name, args, _) = ty else {
        return None;
    };
    let path = module_path(index, &home.module)?;
    let text = snapshot.read(&path).ok()?;
    let root = parse_module(&text).syntax();
    let alias = (root.children())
        .filter(|n| n.kind() == SyntaxKind::TypeAliasDecl)
        .find(|n| token(n, SyntaxKind::UpperIdent).as_deref() == Some(name.as_str()));
    let written = alias.and_then(|alias| {
        (alias.descendants())
            .filter(|n| n.kind() == SyntaxKind::FieldType)
            .find(|f| token(f, SyntaxKind::LowerIdent).as_deref() == Some(field))
    });
    match written {
        Some(declaration) => Some(Location {
            path: path.clone(),
            range: code_range(&declaration)?,
        }),
        // An extensible alias's other fields are its argument's.
        None => args
            .iter()
            .find_map(|(_, arg)| declared(snapshot, index, arg, field)),
    }
}
