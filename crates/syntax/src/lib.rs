//! The lossless Elm syntax tree: lexer, parser and typed views.

mod kind;
mod lexer;
#[allow(dead_code, reason = "the grammar arrives in plan 1.6")]
mod parser;

pub use kind::{ElmLanguage, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};
pub use lexer::{LexError, LexErrorKind, Lexed, Token, lex};
pub use parser::{Parse, SyntaxError, SyntaxErrorKind};
