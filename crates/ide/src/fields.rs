//! Record fields by type: a field written in an access, an
//! accessor, a record, an update or a pattern is declared where the
//! record's type writes it. The record's inferred type keeps the alias
//! it's written as, and the field is that alias's, or one of its
//! arguments'. A variable a record pattern binds is its field's, as
//! elm-language-server has it. A record whose type no alias names has
//! nowhere to point to.

use nomi_base::TextSize;
use nomi_compiler::canonicalize::references::Target;
use nomi_compiler::types::{ExprTypes, error};
use nomi_engine::Snapshot;
use nomi_engine::index::{Index, Indexed};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxNode, child_token, tokens};

use crate::hover::trimmed;
use crate::navigation::{Declaration, Location, module_tree};
use crate::tree::{code_range, declared_type};

/// Where the field at `offset` in `module` is declared, and the tree of
/// the file that declares it.
pub(crate) fn definition(
    snapshot: &Snapshot,
    index: &Index,
    module: &Indexed,
    offset: TextSize,
) -> Option<(Location, SyntaxNode)> {
    // A field's declaration is its own.
    if let Some(declared) = declaration_at(module, offset) {
        return Some((declared, module.root()));
    }
    written(&module.root(), offset)?;
    let (_, types) = module.types(index.compat)?;
    resolve(snapshot, index, module, types, offset)
}

/// Every place the project writes the field at `offset` in `module`,
/// declared where it is: its declaration where `declaration` says so,
/// then each access, accessor, record, update, pattern and variable a
/// pattern binds; none where the offset names no field.
pub(crate) fn references(
    snapshot: &Snapshot,
    index: &Index,
    module: &Indexed,
    offset: TextSize,
    declaration: Declaration,
) -> Option<Vec<Location>> {
    let (declared, root) = definition(snapshot, index, module, offset)?;
    let field = root.covering_element(declared.range).into_node()?;
    let field = field
        .ancestors()
        .find(|n| n.kind() == SyntaxKind::FieldType)?;
    let name = child_token(&field, SyntaxKind::LowerIdent)?;
    let mut found = Vec::new();
    if declaration == Declaration::Include {
        found.push(Location {
            path: declared.path.clone(),
            range: name.text_range(),
        });
    }
    for other in &index.modules {
        let root = other.root();
        let uses: Vec<_> = tokens(&root)
            .filter(|t| t.kind() == SyntaxKind::LowerIdent && t.text() == name.text())
            .filter(|t| written(&root, t.text_range().start()).is_some())
            .collect();
        if uses.is_empty() {
            continue;
        }
        let Some((_, types)) = other.types(index.compat) else {
            continue;
        };
        for token in uses {
            let at = token.text_range().start();
            let resolved = resolve(snapshot, index, other, types, at);
            if resolved.is_some_and(|(location, _)| location == declared) {
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
        (module.root().token_at_offset(offset)).find(|t| t.kind() == SyntaxKind::LowerIdent)?;
    let field = token
        .parent()
        .filter(|p| p.kind() == SyntaxKind::FieldType)?;
    Some(Location {
        path: module.path.clone(),
        range: code_range(&field)?,
    })
}

/// Where the field at `offset` is declared, given `module`'s types, and
/// the tree of the file that declares it.
fn resolve(
    snapshot: &Snapshot,
    index: &Index,
    module: &Indexed,
    types: &ExprTypes,
    offset: TextSize,
) -> Option<(Location, SyntaxNode)> {
    let root = module.root();
    let (record, field) = written(&root, offset)?;
    let range = code_range(&record)?;
    // A node is typed as what its context expects; a variable's alias is
    // its binding's, typed from the annotation.
    let mut ranges = vec![range];
    if record.kind() == SyntaxKind::NameExpr {
        let bound = (module.references.iter())
            .find(|r| r.range.contains_range(range) || range.contains_range(r.range))
            .and_then(|r| match r.target {
                Target::Local(binder) => Some(binder),
                Target::Global { .. } => None,
            })
            .or_else(|| crate::scopes::local(&root, range.start()).map(|(b, _)| b));
        ranges.extend(bound);
    }
    // A type's region holds its node's range, and more trivia before it.
    let typed: Vec<&error::Type> = (types.iter())
        .filter(|(r, _)| ranges.iter().any(|range| r.contains_range(*range)))
        .filter(|(r, _)| trimmed(&root, *r).is_some_and(|r| ranges.contains(&r)))
        .map(|(_, ty)| ty)
        .collect();
    let aliased = |ty: &&&error::Type| match ty {
        error::Type::Alias { .. } => true,
        error::Type::Lambda(arg, _, _) => matches!(**arg, error::Type::Alias { .. }),
        _ => false,
    };
    let ty = typed
        .iter()
        .find(aliased)
        .or_else(|| typed.last())
        .copied()?;
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
fn structural(index: &Index, ty: &error::Type, field: &str) -> Option<(Location, SyntaxNode)> {
    let error::Type::Record(fields, extension) = ty else {
        return None;
    };
    let wanted: Vec<&str> = fields.keys().map(String::as_str).collect();
    let closed = matches!(extension, error::Extension::Closed);
    let mut found: Vec<(Location, SyntaxNode)> = Vec::new();
    for module in &index.modules {
        let root = module.root();
        for record in root.descendants().filter_map(ast::RecordType::cast) {
            let declared: Vec<_> = (record.field_types())
                .filter_map(|f| Some((f.lower_ident_token()?, f)))
                .collect();
            let names: Vec<&str> = declared.iter().map(|(n, _)| n.text()).collect();
            let fits = wanted.iter().all(|w| names.contains(w))
                && (!closed || names.len() == wanted.len());
            if let Some((_, node)) = declared
                .iter()
                .find(|(n, _)| n.text() == field)
                .filter(|_| fits)
            {
                let location = Location {
                    path: module.path.clone(),
                    range: code_range(node.syntax())?,
                };
                found.push((location, root.clone()));
            }
        }
    }
    found.dedup_by(|a, b| a.0 == b.0);
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
        SyntaxKind::Field | SyntaxKind::FieldPattern => parent.parent()?,
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
/// as, or an argument of it; and the tree of the file that declares it.
fn declared(
    snapshot: &Snapshot,
    index: &Index,
    ty: &error::Type,
    field: &str,
) -> Option<(Location, SyntaxNode)> {
    let error::Type::Alias {
        home, name, args, ..
    } = ty
    else {
        return None;
    };
    let (path, root) = module_tree(snapshot, index, &home.module)?;
    let alias = declared_type(&root, SyntaxKind::TypeAliasDecl, name);
    let written = alias.and_then(|alias| {
        (alias.descendants())
            .filter(|n| n.kind() == SyntaxKind::FieldType)
            .find(|f| child_token(f, SyntaxKind::LowerIdent).is_some_and(|t| t.text() == field))
    });
    match written {
        Some(declaration) => {
            let range = code_range(&declaration)?;
            Some((Location { path, range }, root))
        }
        // An extensible alias's other fields are its argument's.
        None => args
            .iter()
            .find_map(|(_, arg)| declared(snapshot, index, arg, field)),
    }
}
