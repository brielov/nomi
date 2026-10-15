//! The lossless Elm syntax tree: lexer, parser and typed views.

pub mod ast;
pub mod elm;
pub mod elm_format;
mod glsl;
mod grammar;
mod invariants;
mod kind;
mod lexer;
mod parser;
mod tree;
mod validation;

pub use grammar::{
    parse_declarations, parse_expr, parse_expressions, parse_module, parse_pattern, parse_type,
};
pub use invariants::check_invariants;
pub use kind::{ElmLanguage, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};
pub use lexer::{LexError, LexErrorKind, Lexed, Token, lex};
pub use parser::{DEPTH_LIMIT, Parse, SyntaxError, SyntaxErrorKind};
pub use rowan::GreenNode;
pub use tree::{
    child_token, child_tokens, int_digits, literal_body, significant_tokens, tokens, trimmed_range,
};
pub use validation::{Source, Tool, UnknownSource, validate};
