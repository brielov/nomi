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

/// What a scope binds, as the binder calls it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bound {
    Let,
    /// A declaration's or a lambda's parameter.
    Parameter,
    Case,
}

/// A name a scope binds.
pub(crate) struct Binding {
    pub(crate) name: String,
    pub(crate) at: TextRange,
    pub(crate) bound: Bound,
    /// A `let` value's definition.
    pub(crate) definition: Option<SyntaxNode>,
}

/// Where `scope` binds `name`.
fn bound_in(scope: &SyntaxNode, name: &str) -> Option<(TextRange, SyntaxNode)> {
    let binding = bound_by(scope).into_iter().find(|b| b.name == name)?;
    Some((binding.at, scope.clone()))
}

/// The names `scope` binds, in order: a case branch's or a lambda's
/// patterns, a declaration's parameters, or a `let`'s declarations.
pub(crate) fn bound_by(scope: &SyntaxNode) -> Vec<Binding> {
    match scope.kind() {
        SyntaxKind::Definition | SyntaxKind::LambdaExpr => {
            pattern_bindings(scope, Bound::Parameter).collect()
        }
        SyntaxKind::CaseBranch => pattern_bindings(scope, Bound::Case).collect(),
        SyntaxKind::LetExpr => {
            let mut found = Vec::new();
            for decl in scope.children() {
                match decl.kind() {
                    SyntaxKind::LetValue => {
                        found.extend(child(&decl, SyntaxKind::Definition).and_then(|definition| {
                            let name = child_token(&definition, SyntaxKind::LowerIdent)?;
                            Some(Binding {
                                name: name.text().to_owned(),
                                at: name.text_range(),
                                bound: Bound::Let,
                                definition: Some(definition),
                            })
                        }));
                    }
                    SyntaxKind::LetDestructure => found.extend(pattern_bindings(&decl, Bound::Let)),
                    _ => {}
                }
            }
            found
        }
        _ => Vec::new(),
    }
}

/// The names the patterns among `node`'s children bind, as `bound`.
fn pattern_bindings(node: &SyntaxNode, bound: Bound) -> impl Iterator<Item = Binding> + use<> {
    (node.children())
        .filter(|c| ast::Pattern::can_cast(c.kind()))
        .flat_map(|pattern| bindings(&pattern))
        .map(move |(name, at)| Binding {
            name,
            at,
            bound,
            definition: None,
        })
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
