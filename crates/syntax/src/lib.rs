//! The lossless Elm syntax tree: lexer, parser and typed views.

mod kind;

pub use kind::{ElmLanguage, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};
