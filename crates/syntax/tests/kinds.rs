use std::collections::BTreeSet;

use nomi_syntax::{SyntaxKind, SyntaxNode};
use ungrammar::{Grammar, Rule};

fn grammar() -> Grammar {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/elm.ungram");
    std::fs::read_to_string(path)
        .unwrap()
        .parse()
        .expect("elm.ungram parses")
}

fn kind_named(name: &str) -> Option<SyntaxKind> {
    SyntaxKind::ALL
        .iter()
        .copied()
        .find(|k| format!("{k:?}") == name)
}

/// A rule that only chooses between nodes is an enum in the typed views.
fn is_enum(rule: &Rule) -> bool {
    matches!(rule, Rule::Alt(alts) if alts.iter().all(|r| matches!(r, Rule::Node(_))))
}

#[test]
fn raw_values_round_trip() {
    for (i, &kind) in SyntaxKind::ALL.iter().enumerate() {
        assert_eq!(kind as usize, i);
        assert_eq!(SyntaxKind::from_raw(kind as u16), Some(kind));
        let raw: rowan::SyntaxKind = kind.into();
        assert_eq!(
            <nomi_syntax::ElmLanguage as rowan::Language>::kind_from_raw(raw),
            kind
        );
    }
    assert_eq!(SyntaxKind::from_raw(SyntaxKind::ALL.len() as u16), None);
}

#[test]
fn fixed_texts_round_trip() {
    let mut texts = BTreeSet::new();
    for &kind in SyntaxKind::ALL {
        if let Some(text) = kind.text() {
            assert!(texts.insert(text), "{text:?} is used twice");
            assert_eq!(SyntaxKind::from_text(text), Some(kind));
            assert!(kind.is_token());
        }
        assert!(!(kind.is_keyword() && kind.is_contextual_keyword()));
        assert_eq!(kind.is_trivia(), kind.is_trivia() && kind.is_token());
    }
    assert_eq!(SyntaxKind::from_text("if"), Some(SyntaxKind::IfKw));
    assert!(SyntaxKind::IfKw.is_keyword());
    assert!(SyntaxKind::AliasKw.is_contextual_keyword());
    assert_eq!(SyntaxKind::from_text("foo"), None);
    assert!(!SyntaxKind::DocComment.is_trivia());
}

#[test]
fn elm_reserved_words_are_exactly_the_keywords() {
    // From elm/compiler's Parse.Variable.reservedWords.
    let reserved = [
        "if", "then", "else", "case", "of", "let", "in", "type", "module", "where", "import",
        "exposing", "as", "port",
    ];
    let keywords: BTreeSet<_> = SyntaxKind::ALL
        .iter()
        .filter(|k| k.is_keyword())
        .map(|k| k.text().unwrap())
        .collect();
    assert_eq!(keywords, reserved.into_iter().collect());
}

#[test]
fn ungram_tokens_have_kinds() {
    let grammar = grammar();
    for token in grammar.tokens() {
        let name = &grammar[token].name;
        let kind = SyntaxKind::ALL
            .iter()
            .find(|k| k.ungram_name() == Some(name.as_str()));
        assert!(kind.is_some(), "ungram token {name:?} has no kind");
    }
}

#[test]
fn ungram_nodes_and_node_kinds_match() {
    let grammar = grammar();
    let mut named = BTreeSet::new();
    for node in grammar.iter() {
        let data = &grammar[node];
        let kind = kind_named(&data.name);
        if is_enum(&data.rule) {
            assert_eq!(kind, None, "{} is an enum, so it has no kind", data.name);
        } else {
            let kind = kind.unwrap_or_else(|| panic!("ungram node {} has no kind", data.name));
            assert!(kind.is_node(), "{} is a token kind", data.name);
            named.insert(kind);
        }
    }
    let unnamed: Vec<_> = SyntaxKind::ALL
        .iter()
        .filter(|k| k.is_node() && !named.contains(k))
        .collect();
    assert_eq!(unnamed, [&SyntaxKind::Missing, &SyntaxKind::Error]);
}

#[test]
fn rowan_trees_carry_the_kinds() {
    let mut builder = rowan::GreenNodeBuilder::new();
    builder.start_node(SyntaxKind::NameExpr.into());
    builder.token(SyntaxKind::LowerIdent.into(), "x");
    builder.token(SyntaxKind::Whitespace.into(), " ");
    builder.finish_node();
    let root = SyntaxNode::new_root(builder.finish());
    assert_eq!(root.kind(), SyntaxKind::NameExpr);
    let kinds: Vec<_> = root.children_with_tokens().map(|e| e.kind()).collect();
    assert_eq!(kinds, [SyntaxKind::LowerIdent, SyntaxKind::Whitespace]);
    assert_eq!(root.text(), "x ");
}
