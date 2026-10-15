//! Walks over syntax trees that every consumer of them needs.

use nomi_base::TextRange;

use crate::{SyntaxKind, SyntaxNode, SyntaxToken};

/// `node`'s child tokens of `kind`, among its own.
#[inline]
pub fn child_tokens(
    node: &SyntaxNode,
    kind: SyntaxKind,
) -> impl Iterator<Item = SyntaxToken> + use<> {
    node.children_with_tokens()
        .filter_map(rowan::NodeOrToken::into_token)
        .filter(move |t| t.kind() == kind)
}

/// `node`'s first child token of `kind`, among its own.
pub fn child_token(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
    child_tokens(node, kind).next()
}

/// Every token under `node`, in order.
pub fn tokens(node: &SyntaxNode) -> impl Iterator<Item = SyntaxToken> + use<> {
    node.descendants_with_tokens()
        .filter_map(rowan::NodeOrToken::into_token)
}

/// The tokens under `node` that aren't trivia, in order.
pub fn significant_tokens(node: &SyntaxNode) -> impl Iterator<Item = SyntaxToken> + use<> {
    tokens(node).filter(|t| !t.kind().is_trivia())
}

/// `node`'s range without the trivia it starts or ends with, or all of it
/// if it is all trivia.
pub fn trimmed_range(node: &SyntaxNode) -> TextRange {
    let mut tokens = significant_tokens(node);
    let Some(first) = tokens.next() else {
        return node.text_range();
    };
    let end = tokens
        .last()
        .unwrap_or_else(|| first.clone())
        .text_range()
        .end();
    TextRange::new(first.text_range().start(), end)
}

/// The text between the quotes of a string, multiline string or
/// character literal, its escapes as written, or between `[glsl|` and
/// `|]` of a shader; `None` for other text, or a literal left open.
pub fn literal_body(literal: &str) -> Option<&str> {
    if let Some(rest) = literal.strip_prefix("\"\"\"") {
        rest.strip_suffix("\"\"\"")
    } else if let Some(rest) = literal.strip_prefix('"') {
        rest.strip_suffix('"')
    } else if let Some(rest) = literal.strip_prefix("[glsl|") {
        rest.strip_suffix("|]")
    } else {
        literal.strip_prefix('\'')?.strip_suffix('\'')
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SyntaxKind as K;

    fn texts(tokens: impl Iterator<Item = SyntaxToken>) -> Vec<String> {
        tokens.map(|t| t.text().to_owned()).collect()
    }

    #[test]
    fn walks_find_tokens() {
        let root = crate::parse_expr("  f -- c\n (x)  ").syntax();
        let call = root.first_child().unwrap();
        assert_eq!(
            texts(tokens(&call)),
            ["f", " ", "-- c", "\n", " ", "(", "x", ")"]
        );
        assert_eq!(texts(significant_tokens(&call)), ["f", "(", "x", ")"]);
        let paren = call.last_child().unwrap();
        assert_eq!(child_token(&paren, K::RParen).unwrap().text(), ")");
        assert_eq!(child_token(&paren, K::LowerIdent), None);
        assert_eq!(trimmed_range(&root), TextRange::new(2.into(), 13.into()));
        let blank = crate::parse_expr("  ").syntax();
        assert_eq!(trimmed_range(&blank), TextRange::new(0.into(), 2.into()));
    }

    #[test]
    fn literal_bodies_lie_between_the_quotes() {
        assert_eq!(literal_body(r#""a\"b""#), Some(r#"a\"b"#));
        assert_eq!(literal_body(r#""""a"b""""#), Some(r#"a"b"#));
        assert_eq!(literal_body(r#""""#), Some(""));
        assert_eq!(literal_body("'\\n'"), Some("\\n"));
        assert_eq!(literal_body("[glsl| x |]"), Some(" x "));
        assert_eq!(literal_body("\"open"), None);
        assert_eq!(literal_body("[glsl| x"), None);
        assert_eq!(literal_body("x"), None);
    }
}
