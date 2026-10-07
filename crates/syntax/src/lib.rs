//! The lossless Elm syntax tree: lexer, parser and typed views.

pub mod ast;
mod grammar;
mod kind;
mod lexer;
#[allow(
    dead_code,
    reason = "the expression grammar (plan 1.8 on) uses the rest"
)]
mod parser;

pub use grammar::{parse_expr, parse_module, parse_pattern, parse_type};
pub use kind::{ElmLanguage, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};
pub use lexer::{LexError, LexErrorKind, Lexed, Token, lex};
pub use parser::{Parse, SyntaxError, SyntaxErrorKind};
