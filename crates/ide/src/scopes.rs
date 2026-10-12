//! Local names by their syntax alone (plan 9.6a), where canonicalization
//! stopped before reaching them: a module elm rejects, for a shadowed
//! name or an annotation too short, is still the editor's to navigate.
//! A name is bound by the nearest pattern around it that binds it, as
//! elm-language-server finds it: a case branch's, a lambda's, a `let`'s
//! declarations, or a declaration's parameters.

use nomi_base::{TextRange, TextSize};
use nomi_syntax::{SyntaxKind, SyntaxNode};

use crate::lint::{bindings, first_token, is_pattern};

/// The binding of the local name at `offset`: where it's bound, and the
/// node it's in scope in. The name may be a use or the binding itself.
pub(crate) fn local(root: &SyntaxNode, offset: TextSize) -> Option<(TextRange, SyntaxNode)> {
    let token = root
        .token_at_offset(offset)
        .find(|t| t.kind() == SyntaxKind::LowerIdent)?;
    let parent = token.parent()?;
    let name = token.text();
    match parent.kind() {
        // A use: unqualified, so possibly a local.
        SyntaxKind::NameExpr if first_token(&parent, SyntaxKind::Dot).is_none() => {
            (parent.ancestors()).find_map(|scope| bound_in(&scope, name))
        }
        _ => {
            // A binding: in the scope that binds it.
            let range = token.text_range();
            (parent.ancestors())
                .find_map(|scope| bound_in(&scope, name))
                .filter(|(binder, _)| *binder == range)
        }
    }
}

/// Every use of the local bound at `binder` in `scope`, and the binding.
pub(crate) fn uses(root: &SyntaxNode, binder: TextRange, scope: &SyntaxNode) -> Vec<TextRange> {
    let mut found = vec![binder];
    for name in scope
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::NameExpr)
    {
        let Some(range) = crate::symbols::code_range(&name) else {
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
    let find = |patterns: &mut dyn Iterator<Item = SyntaxNode>| {
        patterns
            .flat_map(|p| bindings(&p))
            .find(|(n, _)| n == name)
            .map(|(_, range)| (range, scope.clone()))
    };
    match scope.kind() {
        SyntaxKind::CaseBranch | SyntaxKind::LambdaExpr => {
            find(&mut scope.children().filter(|c| is_pattern(c.kind())))
        }
        SyntaxKind::Definition => find(&mut scope.children().filter(|c| is_pattern(c.kind()))),
        SyntaxKind::LetExpr => {
            for decl in scope.children() {
                match decl.kind() {
                    SyntaxKind::LetValue => {
                        let definition =
                            decl.children().find(|c| c.kind() == SyntaxKind::Definition);
                        let declared =
                            definition.and_then(|d| first_token(&d, SyntaxKind::LowerIdent));
                        if let Some(token) = declared.filter(|t| t.text() == name) {
                            return Some((token.text_range(), scope.clone()));
                        }
                    }
                    SyntaxKind::LetDestructure => {
                        let found = find(&mut decl.children().filter(|c| is_pattern(c.kind())));
                        if found.is_some() {
                            return found;
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        _ => None,
    }
}
