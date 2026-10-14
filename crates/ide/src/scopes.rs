//! Local names by their syntax alone, where canonicalization
//! stopped before reaching them: a module elm rejects, for a shadowed
//! name or an annotation too short, is still the editor's to navigate.
//! A name is bound by the nearest pattern around it that binds it, as
//! elm-language-server finds it: a case branch's, a lambda's, a `let`'s
//! declarations, or a declaration's parameters.

use nomi_base::{TextRange, TextSize};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxNode, child_token};

use crate::tree::{child, code_range, last_child_token};

/// The binding of the local name at `offset`: where it's bound, and the
/// node it's in scope in. The name may be a use or the binding itself.
pub(crate) fn local(root: &SyntaxNode, offset: TextSize) -> Option<(TextRange, SyntaxNode)> {
    let token = root
        .token_at_offset(offset)
        .find(|t| t.kind() == SyntaxKind::LowerIdent)?;
    let parent = token.parent()?;
    let name = token.text();
    let bound = parent.ancestors().find_map(|scope| bound_in(&scope, name));
    match parent.kind() {
        // A use: unqualified, so possibly a local; or an update's record.
        SyntaxKind::NameExpr if child_token(&parent, SyntaxKind::Dot).is_none() => bound,
        SyntaxKind::RecordUpdateExpr => bound,
        // A binding: in the scope that binds it.
        _ => bound.filter(|(binder, _)| *binder == token.text_range()),
    }
}

/// Every use of the local bound at `binder` in `scope`, and the binding.
pub(crate) fn uses(root: &SyntaxNode, binder: TextRange, scope: &SyntaxNode) -> Vec<TextRange> {
    let mut found = vec![binder];
    for name in scope
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::NameExpr)
    {
        let Some(range) = code_range(&name) else {
            continue;
        };
        if local(root, range.start()).is_some_and(|(b, _)| b == binder) {
            found.push(range);
        }
    }
    found
}

/// Where `scope` binds `name`: its patterns, a `let`'s declarations, or
/// a declaration's parameters.
fn bound_in(scope: &SyntaxNode, name: &str) -> Option<(TextRange, SyntaxNode)> {
    let find = |node: &SyntaxNode| {
        (node.children().filter(|c| ast::Pattern::can_cast(c.kind())))
            .flat_map(|p| bindings(&p))
            .find(|(n, _)| n == name)
            .map(|(_, range)| (range, scope.clone()))
    };
    match scope.kind() {
        SyntaxKind::CaseBranch | SyntaxKind::LambdaExpr | SyntaxKind::Definition => find(scope),
        SyntaxKind::LetExpr => scope.children().find_map(|decl| match decl.kind() {
            SyntaxKind::LetValue => (child(&decl, SyntaxKind::Definition))
                .and_then(|d| child_token(&d, SyntaxKind::LowerIdent))
                .filter(|token| token.text() == name)
                .map(|token| (token.text_range(), scope.clone())),
            SyntaxKind::LetDestructure => find(&decl),
            _ => None,
        }),
        _ => None,
    }
}

/// The variables `pattern` binds, as tree-sitter's `lower_pattern`s:
/// variables, record fields and `as` names, in order.
pub(crate) fn bindings(pattern: &SyntaxNode) -> Vec<(String, TextRange)> {
    let mut found = Vec::new();
    for node in pattern.descendants() {
        match node.kind() {
            // tree-sitter reads an empty `{}` as a record pattern of one
            // variable with no name, which is never used. It's reported
            // over its parent's parent: a declaration's name and
            // parameters, or the pattern around it.
            SyntaxKind::RecordPattern if child(&node, SyntaxKind::FieldPattern).is_none() => {
                let range = match node.parent().filter(|p| p.kind() == SyntaxKind::Definition) {
                    Some(definition) => {
                        let start = code_range(&definition).map(|r| r.start());
                        let end = (definition.children())
                            .filter(|c| ast::Pattern::can_cast(c.kind()))
                            .last()
                            .and_then(|last| code_range(&last));
                        start
                            .zip(end)
                            .map(|(start, end)| TextRange::new(start, end.end()))
                    }
                    None => code_range(&node),
                };
                if let Some(range) = range {
                    found.push((String::new(), range));
                }
            }
            SyntaxKind::VarPattern | SyntaxKind::FieldPattern => {
                if let Some(name) = child_token(&node, SyntaxKind::LowerIdent) {
                    found.push((name.text().to_owned(), name.text_range()));
                }
            }
            SyntaxKind::AliasPattern => {
                if let Some(name) = last_child_token(&node, SyntaxKind::LowerIdent) {
                    found.push((name.text().to_owned(), name.text_range()));
                }
            }
            _ => {}
        }
    }
    found.sort_by_key(|(_, range)| range.start());
    found
}
