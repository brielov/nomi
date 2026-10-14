//! The symbols a file declares, as elm-language-server 2.10.0 lists them
//! (`documentSymbolProvider.ts`, `symbolTranslator.ts`): one flat list
//! in source order of the module, every value with a definition, `let`'s
//! included, each `let` destructuring by its pattern, each custom type
//! and its variants, and each type alias.
//! A declaration's range leaves out its doc comment, and a value's its
//! signature, as tree-sitter-elm's nodes do.

use std::io;
use std::path::Path;

use nomi_base::TextRange;
use nomi_engine::Snapshot;
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxNode, child_token, parse_module};

use crate::tree::{child, code_range, variant_range, words};

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

/// A name a file declares, and the range of its declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub range: TextRange,
}

/// The symbols of the file at `path`, read as far as its syntax allows.
pub fn document_symbols(snapshot: &Snapshot, path: &Path) -> io::Result<Vec<Symbol>> {
    let text = snapshot.read(path)?;
    Ok(symbols(&parse_module(&text).syntax()))
}

/// The symbols the module `root` is declares.
pub(crate) fn symbols(root: &SyntaxNode) -> Vec<Symbol> {
    let text = |range: TextRange| root.text().slice(range).to_string();
    let symbol = |name, kind, range: Option<TextRange>| {
        Some(Symbol {
            name,
            kind,
            range: range?,
        })
    };
    let mut symbols = Vec::new();
    for node in root.descendants() {
        let named = |kind| child_token(&node, kind).map(|t| t.text().to_owned());
        let found = match node.kind() {
            SyntaxKind::ModuleHeader => child(&node, SyntaxKind::ModuleName)
                .and_then(|name| symbol(words(&name), SymbolKind::Module, code_range(&node))),
            SyntaxKind::ValueDecl | SyntaxKind::LetValue => child(&node, SyntaxKind::Definition)
                .and_then(|definition| {
                    let name = child_token(&definition, SyntaxKind::LowerIdent)?;
                    let range = code_range(&definition);
                    symbol(name.text().to_owned(), SymbolKind::Function, range)
                }),
            // A `let` that destructures: named by its pattern's first part
            // as tree-sitter-elm has it, the whole pattern as written, or a
            // parenthesized one's `(`.
            SyntaxKind::LetDestructure => (node.children().find_map(ast::Pattern::cast))
                .and_then(|pattern| match pattern {
                    ast::Pattern::ParenPattern(_) => Some("(".to_owned()),
                    pattern => code_range(pattern.syntax()).map(text),
                })
                .and_then(|name| symbol(name, SymbolKind::Function, code_range(&node))),
            SyntaxKind::CustomTypeDecl => named(SyntaxKind::UpperIdent)
                .and_then(|name| symbol(name, SymbolKind::Enum, code_range(&node))),
            SyntaxKind::TypeAliasDecl => named(SyntaxKind::UpperIdent)
                .and_then(|name| symbol(name, SymbolKind::Struct, code_range(&node))),
            // A variant is named by its text as written, lines and all, and
            // may run on over a comment.
            SyntaxKind::Variant => variant_range(&node)
                .and_then(|range| symbol(text(range), SymbolKind::EnumMember, Some(range))),
            _ => None,
        };
        symbols.extend(found);
    }
    symbols
}
