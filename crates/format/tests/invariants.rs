//! The fuzz target's property (`fuzz/fuzz_targets/format.rs`), on stable:
//! formatting keeps [`nomi_format::check_invariants`] for edits of the
//! parser's mutation seeds that mostly keep them modules. Without
//! comments, every invariant applies.

use nomi_format::Invariant;
use nomi_syntax::{SyntaxKind, parse_module};
use proptest::prelude::*;

const SEEDS: &[&str] = &[
    include_str!("../../syntax/tests/syntax_mutations/seeds/Accepted.elm"),
    include_str!("../../syntax/tests/syntax_mutations/seeds/AcceptedExpressions.elm"),
    include_str!("../../syntax/tests/syntax_mutations/seeds/AcceptedTypes.elm"),
    include_str!("../../syntax/tests/syntax_mutations/seeds/Declarations.elm"),
    include_str!("../../syntax/tests/syntax_mutations/seeds/Literals.elm"),
    include_str!("../../syntax/tests/syntax_mutations/seeds/Module.elm"),
    include_str!("../../syntax/tests/syntax_mutations/seeds/GLShader.elm"),
    include_str!("../../syntax/tests/syntax_mutations/seeds/InfixDeclaration.elm"),
];

/// What formatting moves and respaces: layout, and the parentheses it
/// drops.
const LAYOUT: &[&str] = &[" ", "\n", "\n ", "\n    ", "\n\n\n", "(", ")"];

const COMMENTS: &[&str] = &["{- c -}", "{-c-}", "-- c\n", "\n-- c\n", "{-| d -}"];

/// `seed` without its comments.
fn uncommented(seed: &str) -> String {
    parse_module(seed)
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| {
            !matches!(
                t.kind(),
                SyntaxKind::LineComment | SyntaxKind::BlockComment | SyntaxKind::DocComment
            )
        })
        .map(|t| t.text().to_owned())
        .collect()
}

#[derive(Clone, Debug)]
enum Edit {
    Delete(usize),
    Insert(&'static str),
}

/// A seed with a few edits at random places, inserting `pieces`.
fn edited(seeds: Vec<String>, pieces: &'static [&'static str]) -> impl Strategy<Value = String> {
    let edit = prop_oneof![
        1 => (1..4_usize).prop_map(Edit::Delete),
        4 => prop::sample::select(pieces).prop_map(Edit::Insert),
    ];
    let edits = prop::collection::vec((any::<prop::sample::Index>(), edit), 1..4);
    (prop::sample::select(seeds), edits).prop_map(|(mut text, edits)| {
        for (at, edit) in edits {
            let mut at = at.index(text.len() + 1);
            while !text.is_char_boundary(at) {
                at -= 1;
            }
            match edit {
                Edit::Delete(len) => {
                    let mut end = (at + len).min(text.len());
                    while !text.is_char_boundary(end) {
                        end += 1;
                    }
                    text.drain(at..end);
                }
                Edit::Insert(piece) => text.insert_str(at, piece),
            }
        }
        text
    })
}

fn seeds() -> Vec<String> {
    SEEDS.iter().map(|&s| s.to_owned()).collect()
}

fn uncommented_seeds() -> Vec<String> {
    SEEDS.iter().map(|s| uncommented(s)).collect()
}

#[test]
fn the_seeds_keep_the_invariants() {
    for seed in uncommented_seeds() {
        assert!(nomi_format::format(&seed).is_some(), "{seed}");
        nomi_format::check_invariants(&seed);
    }
    for seed in SEEDS {
        nomi_format::check_invariants(seed);
    }
}

/// elm-format 0.8.8 prints this comment as `{--}A<   {|- B-}`, which
/// ends at `{--}`; nomi keeps the space after `{--` and with it every
/// invariant. The fuzzer found it.
#[test]
fn a_comment_upstream_ends_early_keeps_every_invariant() {
    let text = "module Example exposing (z)\n\nx =\n{-- }A<   {|- B -} value\n";
    assert_eq!(nomi_format::broken_invariants(text), Vec::new());
}

/// elm-format 0.8.8 reprints this doc comment's Markdown without its
/// nested comment's closing, so the comment swallows `x = 1`; nomi
/// keeps the comment as written. The fuzzer found it.
#[test]
fn a_doc_comment_holding_a_comment_keeps_every_invariant() {
    let text = "module Example exposing (x)\n\n{-|\n\n```emodule exposple exposing (x)\n\n{-|\n\n```emodule exposi-}\n\nx = 1lm\n```\n\n-}\n\nx = 1\n";
    assert_eq!(
        nomi_format::upstream_breaks(text),
        [Invariant::KeepsComments, Invariant::Idempotent]
    );
    assert_eq!(nomi_format::broken_invariants(text), []);
}

/// elm-format drops a shader's carriage returns, which the token check
/// allows.
#[test]
fn a_shader_loses_its_carriage_returns() {
    let text = "module A exposing (s)\n\ns =\n    [glsl|\r\nvoid main () {}\r\n|]\n";
    assert!(nomi_format::upstream_breaks(text).is_empty());
    assert_eq!(nomi_format::broken_invariants(text), []);
}

/// elm-format 0.8.8 formats `(f x) y` with an argument below to `f x y`
/// on one line, then on a second run moves `y` below; the fuzzer found
/// it.
#[test]
fn a_call_of_a_parenthesized_call_need_not_be_idempotent() {
    let text = "module A exposing (x)\n\n\nx =\n    (f x) y\n        e\n";
    assert_eq!(nomi_format::upstream_breaks(text), [Invariant::Idempotent]);
    nomi_format::check_invariants(text);
}

/// `[1..n]` is the call `List.range 1 n`, so it fares as `(f x)` does;
/// the fuzzer found it.
#[test]
fn a_call_of_a_range_need_not_be_idempotent() {
    for text in [
        "module A exposing (x)\n\n\nx =\n    [1..n]\n        e\n",
        "module A exposing (x)\n\n\nx =\n    ([1..n])\n        e\n",
    ] {
        assert_eq!(nomi_format::upstream_breaks(text), [Invariant::Idempotent]);
        nomi_format::check_invariants(text);
    }
}

/// elm-format prints a request to report the declaration in its place;
/// the fuzzer found it.
#[test]
fn a_legacy_infix_spanning_lines_need_not_keep_its_tokens() {
    let text = "x = 1\ninfixl 7 {- a\n -} ===\n";
    assert_eq!(
        nomi_format::upstream_breaks(text),
        Invariant::ALL
            .into_iter()
            .filter(|&i| i != Invariant::KeepsImports)
            .collect::<Vec<_>>()
    );
    nomi_format::check_invariants(text);
}

/// Without a header, elm-format 0.8.8 exposes `Y(..), Y` here, and a
/// second run drops the `Y`; the fuzzer found it.
#[test]
fn a_type_and_an_alias_of_one_name_need_not_be_idempotent() {
    let text = "type alias Y = ()\ntype Y = Y0\n";
    assert_eq!(nomi_format::upstream_breaks(text), [Invariant::Idempotent]);
    nomi_format::check_invariants(text);
}

/// elm-format prints `Infinity`, which Elm doesn't define; nomi prints
/// a literal that is as large. The fuzzer found it.
#[test]
fn a_float_too_large_for_a_double_keeps_every_invariant() {
    let text = "module A exposing (x)\n\n\nx =\n    ( 1e999, -1e999 )\n";
    assert_eq!(
        nomi_format::format(text).as_deref(),
        Some("module A exposing (x)\n\n\nx =\n    ( 1.0e309, -1.0e309 )\n")
    );
    assert_eq!(nomi_format::broken_invariants(text), Vec::new());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn edited_uncommented_seeds_keep_every_invariant(text in edited(uncommented_seeds(), LAYOUT)) {
        nomi_format::check_invariants(&text);
    }

    #[test]
    fn commented_seeds_keep_the_invariants(text in edited(seeds(), COMMENTS)) {
        nomi_format::check_invariants(&text);
    }
}
