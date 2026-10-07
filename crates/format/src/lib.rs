//! The formatter: elm-format 0.8.8's output for every module it accepts
//! (docs/architecture.md §5).
//!
//! Rendering arrives in pieces (plan 2.2 to 2.13); until a piece lands,
//! its part of the module keeps the source's text, so a module that is
//! already formatted comes back unchanged from the start.

// Rendering takes up the rest of these from plan 2.9 to 2.11.
#[cfg_attr(not(test), expect(dead_code))]
mod block;
mod lower;
mod render;
#[cfg_attr(not(test), expect(dead_code))]
mod structure;
mod tree;

use nomi_base::TextSize;
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxNode, Tool, parse_module, validate};

use tree::{Module, TopLevelStructure};

/// `text` formatted as elm-format does, or `None` where elm-format
/// rejects it.
pub fn format(text: &str) -> Option<String> {
    let parse = parse_module(text);
    if !parse.errors().is_empty() || !validate(&parse, Tool::ElmFormat).is_empty() {
        return None;
    }
    let root = parse.syntax();
    let module = lower::module(&root, text);
    let body = &text[usize::from(body_start(&root, &module, text))..];
    Some(render::module(&module, body))
}

/// Where the body's first item starts in `text`: its first declaration,
/// or the comment before it that the body holds first. Until plan 2.11
/// renders declarations, the body keeps its source text from there.
fn body_start(root: &SyntaxNode, module: &Module, text: &str) -> TextSize {
    let decl_start = ast::Module::cast(root.clone())
        .and_then(|m| m.decls().next())
        .and_then(|d| {
            // A node's range takes in the trivia before it.
            d.syntax()
                .descendants_with_tokens()
                .filter_map(|e| e.into_token())
                .find(|t| !t.kind().is_trivia())
        })
        .map_or(TextSize::of(text), |t| t.text_range().start());
    let leading = module
        .body
        .iter()
        .take_while(|item| matches!(item, TopLevelStructure::BodyComment(_)))
        .count();
    if leading == 0 {
        return decl_start;
    }
    // The body's comments before its first declaration are the last ones
    // before it.
    let comments: Vec<TextSize> = root
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .take_while(|t| t.text_range().start() < decl_start)
        .filter(|t| matches!(t.kind(), SyntaxKind::LineComment | SyntaxKind::BlockComment))
        .map(|t| t.text_range().start())
        .collect();
    comments[comments.len() - leading]
}

#[cfg(test)]
mod tests;
