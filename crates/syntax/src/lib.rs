//! The lossless Elm syntax tree: lexer, parser and typed views.

mod kind;
mod lexer;

pub use kind::{ElmLanguage, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};
pub use lexer::{LexError, LexErrorKind, Lexed, Token, lex};
