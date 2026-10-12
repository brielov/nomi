//! The symbols a file declares, as elm-language-server 2.10.0 lists them
//! (`documentSymbolProvider.ts`, `symbolTranslator.ts`): one flat list
//! in source order of the module, every value with a definition, `let`'s
//! included, each custom type and its variants, and each type alias.
//! A declaration's range leaves out its doc comment, and a value's its
//! signature, as tree-sitter-elm's nodes do.

use std::io;
use std::path::Path;

use nomi_base::TextRange;
use nomi_engine::Snapshot;
use nomi_syntax::{SyntaxKind, SyntaxNode, parse_module};

/// What a symbol is, as elm-language-server names it to the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    Module,
    /// A value, which elm-language-server calls a function either way.
    Function,
    /// A custom type.
    Enum,
    /// A type alias.
    Struct,
    /// A custom type's variant.
    EnumMember,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub range: TextRange,
}

/// The symbols of the file at `path`, read as far as its syntax allows.
pub fn document_symbols(snapshot: &Snapshot, path: &Path) -> io::Result<Vec<Symbol>> {
    let text = snapshot.read(path)?;
    let parse = parse_module(&text);
    Ok(symbols(&parse.syntax()))
}

fn symbols(root: &SyntaxNode) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for node in root.descendants() {
        let symbol = match node.kind() {
            SyntaxKind::ModuleHeader => child(&node, SyntaxKind::ModuleName)
                .map(|name| (words(&name), SymbolKind::Module, node.clone())),
            SyntaxKind::ValueDecl | SyntaxKind::LetValue => child(&node, SyntaxKind::Definition)
                .and_then(|definition| {
                    let name = token(&definition, SyntaxKind::LowerIdent)?;
                    Some((name, SymbolKind::Function, definition))
                }),
            SyntaxKind::CustomTypeDecl => token(&node, SyntaxKind::UpperIdent)
                .map(|name| (name, SymbolKind::Enum, node.clone())),
            SyntaxKind::TypeAliasDecl => token(&node, SyntaxKind::UpperIdent)
                .map(|name| (name, SymbolKind::Struct, node.clone())),
            SyntaxKind::Variant => Some((words(&node), SymbolKind::EnumMember, node.clone())),
            _ => None,
        };
        if let Some((name, kind, node)) = symbol
            && let Some(range) = code_range(&node)
        {
            // A variant's node may run on over a comment, name and all.
            if kind == SymbolKind::EnumMember
                && let Some(extent) = variant_range(&node).filter(|e| *e != range)
            {
                let text = root.text().slice(extent).to_string();
                symbols.push(Symbol {
                    name: text,
                    kind,
                    range: extent,
                });
                continue;
            }
            symbols.push(Symbol { name, kind, range });
        }
    }
    symbols
}

pub(crate) fn child(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxNode> {
    node.children().find(|c| c.kind() == kind)
}

/// The text of `node`'s first token of `kind` among its own.
pub(crate) fn token(node: &SyntaxNode, kind: SyntaxKind) -> Option<String> {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == kind)
        .map(|t| t.text().to_owned())
}

/// `node`'s text without trivia, as tree-sitter's `text` of a name or a
/// variant reads on one line.
pub(crate) fn words(node: &SyntaxNode) -> String {
    let mut out = String::new();
    let mut gap = false;
    for token in node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
    {
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
/// its last that isn't trivia.
pub(crate) fn code_range(node: &SyntaxNode) -> Option<TextRange> {
    let mut tokens = node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia() && t.kind() != SyntaxKind::DocComment);
    let first = tokens.next()?;
    let last = tokens.last().unwrap_or_else(|| first.clone());
    Some(TextRange::new(
        first.text_range().start(),
        last.text_range().end(),
    ))
}

/// A top-level declaration's range as tree-sitter-elm's node has it:
/// [`code_range`], and for a custom type, the line comments after it
/// where a block comment comes next, through blank lines.
pub(crate) fn declaration_range(node: &SyntaxNode) -> Option<TextRange> {
    let range = code_range(node)?;
    // A custom type's last variant might go on with arguments, so the
    // comments after it wait to see; an alias's don't.
    if node.kind() != SyntaxKind::CustomTypeDecl {
        return Some(range);
    }
    let last = node
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.text_range().end() == range.end())
        .last()?;
    let mut comments = range.end();
    let mut next = last.next_token();
    while let Some(token) = next {
        match token.kind() {
            SyntaxKind::Whitespace | SyntaxKind::Newline => {}
            SyntaxKind::LineComment => comments = token.text_range().end(),
            // Its scanner ends the declaration at the next code, before
            // the comments that lead to it, but past those that lead to a
            // block comment, which it can't look past.
            SyntaxKind::BlockComment | SyntaxKind::DocComment => {
                return Some(TextRange::new(range.start(), comments));
            }
            _ => break,
        }
        next = token.next_token();
    }
    Some(range)
}

/// A variant's extent as tree-sitter-elm's node has it: its code, and
/// the line comments after it where another variant follows, which its
/// node takes in.
pub(crate) fn variant_range(node: &SyntaxNode) -> Option<TextRange> {
    let range = code_range(node)?;
    let mut end = range.end();
    let mut comments = range.end();
    let mut next = node.last_token().and_then(|t| t.next_token());
    while let Some(token) = next {
        match token.kind() {
            SyntaxKind::Whitespace | SyntaxKind::Newline => {}
            SyntaxKind::LineComment => comments = token.text_range().end(),
            SyntaxKind::Pipe => {
                end = comments;
                break;
            }
            _ => break,
        }
        next = token.next_token();
    }
    Some(TextRange::new(range.start(), end))
}
