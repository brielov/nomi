//! Typed views over the syntax tree. Each view wraps a [`SyntaxNode`] of
//! its kind; accessors return `None` where the parser recorded a missing
//! or malformed child. The views are generated from `elm.ungram`.

mod generated;

use std::marker::PhantomData;

pub use generated::*;

use crate::{SyntaxKind, SyntaxNode, SyntaxToken};

/// A typed view of one node kind, or of a choice between kinds.
pub trait AstNode: Sized {
    fn can_cast(kind: SyntaxKind) -> bool;

    fn cast(syntax: SyntaxNode) -> Option<Self>;

    fn syntax(&self) -> &SyntaxNode;
}

/// The children of a node that cast to `N`, in order.
#[derive(Clone, Debug)]
pub struct AstChildren<N> {
    inner: rowan::SyntaxNodeChildren<crate::ElmLanguage>,
    ph: PhantomData<N>,
}

impl<N: AstNode> Iterator for AstChildren<N> {
    type Item = N;

    fn next(&mut self) -> Option<N> {
        self.inner.find_map(N::cast)
    }
}

mod support {
    use super::{AstChildren, AstNode, PhantomData, SyntaxKind, SyntaxNode, SyntaxToken};

    pub(super) fn child<N: AstNode>(parent: &SyntaxNode) -> Option<N> {
        parent.children().find_map(N::cast)
    }

    pub(super) fn children<N: AstNode>(parent: &SyntaxNode) -> AstChildren<N> {
        AstChildren {
            inner: parent.children(),
            ph: PhantomData,
        }
    }

    /// The node right after the first `token`, if it is an `N`; `None`
    /// when the token is absent or the slot holds a Missing or Error node.
    pub(super) fn child_after<N: AstNode>(parent: &SyntaxNode, token: SyntaxKind) -> Option<N> {
        parent
            .children_with_tokens()
            .skip_while(|e| e.kind() != token)
            .find_map(|e| e.into_node())
            .and_then(N::cast)
    }

    /// The node right before the first `token`, if it is an `N`; the last
    /// node when the token is absent.
    pub(super) fn child_before<N: AstNode>(parent: &SyntaxNode, token: SyntaxKind) -> Option<N> {
        parent
            .children_with_tokens()
            .take_while(|e| e.kind() != token)
            .filter_map(|e| e.into_node())
            .last()
            .and_then(N::cast)
    }

    pub(super) fn token(parent: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
        parent
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == kind)
    }
}
