//! Code lenses as elm-language-server 2.10.0 shows them
//! (`codeLensProvider.ts`): over each top-level declaration, whether the
//! module exposes it, and over each declaration, its reference count,
//! both resolved later. The lenses come in its order: the exposing ones
//! in declaration order, then the counts of types and ports, of values at
//! any depth, `let`'s included, and of the module's name.
//!
//! A lens sits where tree-sitter-elm's node is: a value's signature when
//! it's the value's previous named node, so a comment between the two
//! moves it to the definition.

use std::io;
use std::path::Path;

use nomi_base::TextRange;
use nomi_engine::Snapshot;
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken, child_token, parse_module, tokens};

use crate::tree::{child, code_range, code_text, declaration_range};

/// What a name the module may expose names: a value or port, or a type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exposable {
    Value,
    Type,
}

/// What a lens shows once resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LensKind {
    /// Whether the module exposes `name`.
    Exposed { name: String, what: Exposable },
    /// How many references the name at the lens's start has.
    References,
}

/// A lens over a declaration, and what it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lens {
    pub kind: LensKind,
    pub range: TextRange,
}

/// The lenses of the file at `path`, read as far as its syntax allows.
pub fn code_lenses(snapshot: &Snapshot, path: &Path) -> io::Result<Vec<Lens>> {
    let text = snapshot.read(path)?;
    let root = parse_module(&text).syntax();
    let count = |range: TextRange| Lens {
        kind: LensKind::References,
        range,
    };
    let mut lenses = Vec::new();
    for node in root.children() {
        let lens = match node.kind() {
            SyntaxKind::ValueDecl => value_name(&node)
                .map(|name| (name, Exposable::Value))
                .zip(value_range(&node)),
            SyntaxKind::CustomTypeDecl | SyntaxKind::TypeAliasDecl => declared_name(&node)
                .map(|name| (name, Exposable::Type))
                .zip(declaration_range(&node)),
            SyntaxKind::PortDecl => declared_name(&node)
                .map(|name| (name, Exposable::Value))
                .zip(code_range(&node)),
            _ => None,
        };
        lenses.extend(lens.map(|((name, what), range)| Lens {
            kind: LensKind::Exposed {
                name: name.text().to_owned(),
                what,
            },
            range,
        }));
    }
    for node in root.children() {
        if let Some(name) = declared_name(&node) {
            lenses.push(count(name.text_range()));
        }
    }
    for node in root.descendants() {
        if matches!(node.kind(), SyntaxKind::ValueDecl | SyntaxKind::LetValue) {
            lenses.extend(value_range(&node).map(count));
        }
    }
    let module_name = (root.children().find_map(ast::ModuleHeader::cast))
        .and_then(|header| header.module_name())
        .and_then(|name| code_range(name.syntax()));
    lenses.extend(module_name.map(count));
    Ok(lenses)
}

/// Whether the file at `path` exposes `name`: as elm-language-server
/// reads it, an exposed type matches by its text, so `Shape (..)` isn't
/// `Shape(..)`.
pub fn is_exposed(
    snapshot: &Snapshot,
    path: &Path,
    name: &str,
    what: Exposable,
) -> io::Result<bool> {
    let text = snapshot.read(path)?;
    Ok(exposes(&parse_module(&text).syntax(), name, what))
}

/// [`is_exposed`] of a module already parsed.
pub(crate) fn exposes(root: &SyntaxNode, name: &str, what: Exposable) -> bool {
    let list = (root.children().find_map(ast::ModuleHeader::cast))
        .and_then(|header| header.exposing_list());
    let Some(list) = list else {
        return false;
    };
    if list.dot_dot_token().is_some() {
        return true;
    }
    let wanted = match what {
        Exposable::Value => SyntaxKind::ExposedValue,
        Exposable::Type => SyntaxKind::ExposedType,
    };
    (list.syntax().descendants())
        .filter(|n| n.kind() == wanted)
        .filter_map(|n| code_text(&n))
        .any(|written| {
            written == name
                || (what == Exposable::Type && written.strip_suffix("(..)") == Some(name))
        })
}

/// The name a value declaration defines.
fn value_name(node: &SyntaxNode) -> Option<SyntaxToken> {
    child_token(
        &child(node, SyntaxKind::Definition)?,
        SyntaxKind::LowerIdent,
    )
}

/// The name a type, alias or port declares.
fn declared_name(node: &SyntaxNode) -> Option<SyntaxToken> {
    match node.kind() {
        SyntaxKind::CustomTypeDecl | SyntaxKind::TypeAliasDecl => {
            child_token(node, SyntaxKind::UpperIdent)
        }
        SyntaxKind::PortDecl => child_token(node, SyntaxKind::LowerIdent),
        _ => None,
    }
}

/// Where a value's lenses sit: its signature, unless a comment comes
/// between it and the definition, else the definition.
fn value_range(node: &SyntaxNode) -> Option<TextRange> {
    let definition = code_range(&child(node, SyntaxKind::Definition)?)?;
    let Some(signature) = child(node, SyntaxKind::Signature).and_then(|s| code_range(&s)) else {
        return Some(definition);
    };
    let commented = tokens(node)
        .filter(|t| {
            signature.end() <= t.text_range().start() && t.text_range().end() <= definition.start()
        })
        .any(|t| {
            t.kind().is_trivia()
                && !matches!(t.kind(), SyntaxKind::Whitespace | SyntaxKind::Newline)
        });
    Some(if commented { definition } else { signature })
}
