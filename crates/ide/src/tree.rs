//! Walks over the tree that the editor's answers share, and the tree as
//! tree-sitter-elm's nodes read, which elm-language-server answers from:
//! a node's range and text leave out the trivia around it and its doc
//! comment, and a declaration's take in the comments its scanner lets run
//! on.

use nomi_base::{TextRange, TextSize};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{
    SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken, child_token, child_tokens, tokens,
};

/// The smallest node covering `range`, which `root` holds.
pub(crate) fn covering_node(root: &SyntaxNode, range: TextRange) -> Option<SyntaxNode> {
    match root.covering_element(range) {
        SyntaxElement::Node(node) => Some(node),
        SyntaxElement::Token(token) => token.parent(),
    }
}

/// `node`'s first child node of `kind`.
pub(crate) fn child(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxNode> {
    node.children().find(|c| c.kind() == kind)
}

/// `node`'s last child token of `kind`, among its own.
pub(crate) fn last_child_token(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
    child_tokens(node, kind).last()
}

/// `token`, or else the first after it that isn't trivia.
pub(crate) fn skip_trivia(token: SyntaxToken) -> Option<SyntaxToken> {
    std::iter::successors(Some(token), SyntaxToken::next_token).find(|t| !t.kind().is_trivia())
}

/// The custom type or alias, as `kind` says, that the module `root` is
/// declares as `name`.
pub(crate) fn declared_type(root: &SyntaxNode, kind: SyntaxKind, name: &str) -> Option<SyntaxNode> {
    (root.children())
        .filter(|n| n.kind() == kind)
        .find(|n| child_token(n, SyntaxKind::UpperIdent).is_some_and(|t| t.text() == name))
}

/// `node`'s text without trivia, as tree-sitter's `text` of a name or a
/// variant reads on one line.
pub(crate) fn words(node: &SyntaxNode) -> String {
    let mut out = String::new();
    let mut gap = false;
    for token in tokens(node) {
        if token.kind().is_trivia() {
            gap = !out.is_empty();
        } else {
            if gap {
                out.push(' ');
                gap = false;
            }
            out.push_str(token.text());
        }
    }
    out
}

/// From `node`'s first token that is neither trivia nor a doc comment to
/// its last; none where it has no such token.
pub(crate) fn code_range(node: &SyntaxNode) -> Option<TextRange> {
    let (start, end) = code_tokens(node)?;
    Some(TextRange::new(
        start.text_range().start(),
        end.text_range().end(),
    ))
}

/// The first and last of `node`'s tokens that are neither trivia nor a
/// doc comment, found from its ends.
fn code_tokens(node: &SyntaxNode) -> Option<(SyntaxToken, SyntaxToken)> {
    let (first, last) = (node.first_token()?, node.last_token()?);
    let start = code_token(first, &last, SyntaxToken::next_token)?;
    let end = code_token(last, &start, SyntaxToken::prev_token)?;
    Some((start, end))
}

/// The first token from `token` on, by `step`, that is neither trivia
/// nor a doc comment, stopping at `bound`.
fn code_token(
    mut token: SyntaxToken,
    bound: &SyntaxToken,
    step: fn(&SyntaxToken) -> Option<SyntaxToken>,
) -> Option<SyntaxToken> {
    loop {
        if !token.kind().is_trivia() && token.kind() != SyntaxKind::DocComment {
            return Some(token);
        }
        if token == *bound {
            return None;
        }
        token = step(&token)?;
    }
}

/// `node`'s text over its [`code_range`]: tree-sitter's `text`.
pub(crate) fn code_text(node: &SyntaxNode) -> Option<String> {
    let range = code_range(node)?;
    let start = range.start() - node.text_range().start();
    Some(node.text().slice(start..start + range.len()).to_string())
}

/// `node` as `text`, its tree's text, writes it, over its [`code_range`]:
/// [`code_text`] without the copy, empty where it has no code.
pub(crate) fn written<'t>(text: &'t str, node: &SyntaxNode) -> &'t str {
    code_range(node).map_or("", |r| &text[r])
}

/// A top-level declaration's range as tree-sitter-elm's node has it:
/// [`code_range`], and for a custom type, the line comments after it
/// where a block comment comes next, through blank lines.
pub(crate) fn declaration_range(node: &SyntaxNode) -> Option<TextRange> {
    let (start, end) = code_tokens(node)?;
    let range = TextRange::new(start.text_range().start(), end.text_range().end());
    // A custom type's last variant might go on with arguments, so the
    // comments after it wait to see; an alias's don't.
    if node.kind() != SyntaxKind::CustomTypeDecl {
        return Some(range);
    }
    // Its scanner ends the declaration at the next code, before the
    // comments that lead to it, but past those that lead to a block
    // comment, which it can't look past.
    let stops = [SyntaxKind::BlockComment, SyntaxKind::DocComment];
    Some(TextRange::new(range.start(), comments_until(&end, &stops)))
}

/// A variant's extent as tree-sitter-elm's node has it: its code, and
/// the line comments after it where another variant or a block comment
/// follows, which its node takes in, as the last variant's declaration
/// does.
pub(crate) fn variant_range(node: &SyntaxNode) -> Option<TextRange> {
    let range = code_range(node)?;
    let stops = [
        SyntaxKind::Pipe,
        SyntaxKind::BlockComment,
        SyntaxKind::DocComment,
    ];
    let end = comments_until(&node.last_token()?, &stops);
    Some(TextRange::new(range.start(), end))
}

/// Where the line comments after `token` end, through blank lines, when
/// a token of one of `stops` comes next; else where `token` ends.
fn comments_until(token: &SyntaxToken, stops: &[SyntaxKind]) -> TextSize {
    let mut end = token.text_range().end();
    let mut next = token.next_token();
    while let Some(after) = next {
        match after.kind() {
            SyntaxKind::Whitespace | SyntaxKind::Newline => {}
            SyntaxKind::LineComment => end = after.text_range().end(),
            kind if stops.contains(&kind) => return end,
            _ => break,
        }
        next = after.next_token();
    }
    token.text_range().end()
}

/// A function type's arguments, in order.
pub(crate) fn arguments(ty: &SyntaxNode) -> Vec<SyntaxNode> {
    let mut found = Vec::new();
    let mut ty = ty.clone();
    while ty.kind() == SyntaxKind::FunctionType {
        let mut parts = ty.children().filter(|c| ast::Type::can_cast(c.kind()));
        let (Some(argument), Some(result)) = (parts.next(), parts.next()) else {
            break;
        };
        found.push(argument);
        ty = result;
    }
    found
}

/// A function type's result.
pub(crate) fn result_type(ty: &SyntaxNode) -> SyntaxNode {
    let mut ty = ty.clone();
    while ty.kind() == SyntaxKind::FunctionType {
        let Some(result) = ty
            .children()
            .filter(|c| ast::Type::can_cast(c.kind()))
            .nth(1)
        else {
            break;
        };
        ty = result;
    }
    ty
}

/// An import as written: its node, its module's name, and its alias.
pub(crate) struct WrittenImport {
    pub(crate) node: ast::Import,
    pub(crate) module: String,
    pub(crate) alias: Option<String>,
}

/// The imports of the module `root` is, each that names its module, in
/// order.
pub(crate) fn written_imports(root: &SyntaxNode) -> impl Iterator<Item = WrittenImport> + use<> {
    root.children()
        .filter_map(ast::Import::cast)
        .filter_map(|node| {
            let module = words(node.module_name()?.syntax());
            let alias = node.import_alias().and_then(|a| a.upper_ident_token());
            Some(WrittenImport {
                alias: alias.map(|a| a.text().to_owned()),
                node,
                module,
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODULE: &str = "module Main exposing (..)

import Html as H exposing (div)
import List


{-| A shape. -}
type Shape
    = Circle -- round
    -- then
    | Square


f x = x
";

    #[test]
    fn ranges_leave_out_trivia_and_doc_comments() {
        let root = nomi_syntax::parse_module(MODULE).syntax();
        let text = |range: TextRange| &MODULE[range];
        let shape = declared_type(&root, SyntaxKind::CustomTypeDecl, "Shape").unwrap();
        assert!(text(code_range(&shape).unwrap()).starts_with("type Shape"));
        assert!(code_text(&shape).unwrap().ends_with("| Square"));
        let circle = child(&shape, SyntaxKind::Variant).unwrap();
        assert_eq!(
            text(variant_range(&circle).unwrap()),
            "Circle -- round\n    -- then"
        );
        let space = root
            .token_at_offset(TextSize::new(25))
            .right_biased()
            .unwrap();
        assert_eq!(skip_trivia(space).unwrap().text(), "import");
    }

    #[test]
    fn imports_are_read_as_written() {
        let root = nomi_syntax::parse_module(MODULE).syntax();
        let imports: Vec<(String, Option<String>)> = written_imports(&root)
            .map(|i| (i.module, i.alias))
            .collect();
        assert_eq!(
            imports,
            [
                ("Html".to_owned(), Some("H".to_owned())),
                ("List".to_owned(), None)
            ]
        );
        let alias = written_imports(&root).next().unwrap().node;
        let name = last_child_token(
            alias.import_alias().unwrap().syntax(),
            SyntaxKind::UpperIdent,
        );
        assert_eq!(name.unwrap().text(), "H");
    }
}
