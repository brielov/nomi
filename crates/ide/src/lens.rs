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
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken, parse_module};

use crate::symbols::{child, code_range, declaration_range};

/// What a lens shows once resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LensKind {
    /// Whether the module exposes `name`, a value or port if `function`,
    /// else a type.
    Exposed { name: String, function: bool },
    /// How many references the name at the lens's start has.
    References,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lens {
    pub kind: LensKind,
    pub range: TextRange,
}

/// The lenses of the file at `path`, read as far as its syntax allows.
pub fn code_lenses(snapshot: &Snapshot, path: &Path) -> io::Result<Vec<Lens>> {
    let text = snapshot.read(path)?;
    let root = parse_module(&text).syntax();
    let mut lenses = Vec::new();
    for node in root.children() {
        let exposed = |name: String, function: bool, range: Option<TextRange>| {
            range.map(|range| Lens {
                kind: LensKind::Exposed { name, function },
                range,
            })
        };
        let lens = match node.kind() {
            SyntaxKind::ValueDecl => value_name(&node)
                .and_then(|name| exposed(name.text().to_owned(), true, value_range(&node))),
            SyntaxKind::CustomTypeDecl | SyntaxKind::TypeAliasDecl => {
                name(&node, SyntaxKind::UpperIdent).and_then(|name| {
                    exposed(name.text().to_owned(), false, declaration_range(&node))
                })
            }
            SyntaxKind::PortDecl => name(&node, SyntaxKind::LowerIdent)
                .and_then(|name| exposed(name.text().to_owned(), true, code_range(&node))),
            _ => None,
        };
        lenses.extend(lens);
    }
    let count = |range: TextRange| Lens {
        kind: LensKind::References,
        range,
    };
    for node in root.children() {
        let name = match node.kind() {
            SyntaxKind::CustomTypeDecl | SyntaxKind::TypeAliasDecl => {
                name(&node, SyntaxKind::UpperIdent)
            }
            SyntaxKind::PortDecl => name(&node, SyntaxKind::LowerIdent),
            _ => None,
        };
        lenses.extend(name.map(|name| count(name.text_range())));
    }
    for node in root.descendants() {
        if matches!(node.kind(), SyntaxKind::ValueDecl | SyntaxKind::LetValue) {
            lenses.extend(value_range(&node).map(count));
        }
    }
    let module_name = (root.children())
        .find(|n| n.kind() == SyntaxKind::ModuleHeader)
        .and_then(|header| child(&header, SyntaxKind::ModuleName))
        .and_then(|name| code_range(&name));
    lenses.extend(module_name.map(count));
    Ok(lenses)
}

/// Whether the file at `path` exposes `name`, a value or port if
/// `function`, else a type: as elm-language-server reads it, an exposed
/// type matches by its text, so `Shape (..)` isn't `Shape(..)`.
pub fn is_exposed(
    snapshot: &Snapshot,
    path: &Path,
    name: &str,
    function: bool,
) -> io::Result<bool> {
    let text = snapshot.read(path)?;
    let root = parse_module(&text).syntax();
    let Some(list) = (root.children())
        .find(|n| n.kind() == SyntaxKind::ModuleHeader)
        .and_then(|header| child(&header, SyntaxKind::ExposingList))
    else {
        return Ok(false);
    };
    let everything = (list.children_with_tokens())
        .filter_map(|e| e.into_token())
        .any(|t| t.kind() == SyntaxKind::DotDot);
    if everything {
        return Ok(true);
    }
    let wanted = if function {
        SyntaxKind::ExposedValue
    } else {
        SyntaxKind::ExposedType
    };
    let as_written =
        |node: &SyntaxNode| code_range(node).map(|r| &text[std::ops::Range::<usize>::from(r)]);
    Ok((list.descendants())
        .filter(|n| n.kind() == wanted)
        .filter_map(|n| as_written(&n))
        .any(|written| written == name || (!function && written == format!("{name}(..)"))))
}

/// The name a value declaration defines.
fn value_name(node: &SyntaxNode) -> Option<SyntaxToken> {
    name(
        &child(node, SyntaxKind::Definition)?,
        SyntaxKind::LowerIdent,
    )
}

/// Where a value's lenses sit: its signature, unless a comment comes
/// between it and the definition, else the definition.
fn value_range(node: &SyntaxNode) -> Option<TextRange> {
    let definition = code_range(&child(node, SyntaxKind::Definition)?)?;
    let Some(signature) = child(node, SyntaxKind::Signature).and_then(|s| code_range(&s)) else {
        return Some(definition);
    };
    let commented = (node.descendants_with_tokens())
        .filter_map(|e| e.into_token())
        .filter(|t| {
            signature.end() <= t.text_range().start() && t.text_range().end() <= definition.start()
        })
        .any(|t| {
            t.kind().is_trivia()
                && !matches!(t.kind(), SyntaxKind::Whitespace | SyntaxKind::Newline)
        });
    Some(if commented { definition } else { signature })
}

/// `node`'s first token of `kind` among its own.
fn name(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == kind)
}
