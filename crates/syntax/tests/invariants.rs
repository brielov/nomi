//! The fuzz target's property (`fuzz/fuzz_targets/syntax.rs`), on stable:
//! every entry point keeps [`nomi_syntax::check_invariants`] for edits of
//! the mutation seeds and for soups of Elm's tokens.

use proptest::prelude::*;

const SEEDS: &[&str] = &[
    include_str!("syntax_mutations/seeds/Accepted.elm"),
    include_str!("syntax_mutations/seeds/AcceptedExpressions.elm"),
    include_str!("syntax_mutations/seeds/AcceptedTypes.elm"),
    include_str!("syntax_mutations/seeds/Declarations.elm"),
    include_str!("syntax_mutations/seeds/Literals.elm"),
    include_str!("syntax_mutations/seeds/Module.elm"),
    include_str!("syntax_mutations/seeds/GLShader.elm"),
    include_str!("syntax_mutations/seeds/InfixDeclaration.elm"),
];

/// Pieces of Elm: keywords, punctuation, literals and layout, the
/// beginnings of things left open, and characters the lexer treats apart.
const FRAGMENTS: &[&str] = &[
    "module",
    "port",
    "effect",
    "exposing",
    "where",
    "import",
    "as",
    "type",
    "alias",
    "infix",
    "left",
    "right",
    "non",
    "if",
    "then",
    "else",
    "case",
    "of",
    "let",
    "in",
    "x",
    "Maybe.Just",
    "_",
    "a.b",
    ".field",
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
    ",",
    "|",
    "=",
    ":",
    "->",
    "\\",
    "..",
    "-",
    "+",
    "<|",
    "|>",
    "==",
    "::",
    "(+)",
    "1",
    "0x1F",
    "1.5e3",
    "'c'",
    "\"s\"",
    "\"\"\"m\n\"\"\"",
    "\"",
    "'",
    "\"\"\"",
    "{-",
    "-}",
    "{-|",
    "--",
    "[glsl|",
    "|]",
    "void main () {}",
    "\n",
    "\n ",
    "\n    ",
    "\r\n",
    "\t",
    " ",
    "Δ",
    "名",
    "\0",
];

fn soup() -> impl Strategy<Value = String> {
    let fragment = prop::sample::select(FRAGMENTS).prop_map(str::to_owned);
    let any = any::<char>().prop_map(String::from);
    prop::collection::vec(prop_oneof![8 => fragment, 1 => any], 0..80)
        .prop_map(|parts| parts.join(" "))
}

#[derive(Clone, Debug)]
enum Edit {
    Delete(usize),
    Insert(&'static str),
    Duplicate(usize),
}

/// A seed with a few edits at random places, which keeps most of the
/// input well formed around them.
fn edited_seed() -> impl Strategy<Value = String> {
    let edit = prop_oneof![
        (1..40_usize).prop_map(Edit::Delete),
        prop::sample::select(FRAGMENTS).prop_map(Edit::Insert),
        (1..80_usize).prop_map(Edit::Duplicate),
    ];
    let edits = prop::collection::vec((any::<prop::sample::Index>(), edit), 1..6);
    (prop::sample::select(SEEDS), edits).prop_map(|(seed, edits)| {
        let mut text = seed.to_owned();
        for (at, edit) in edits {
            let mut at = at.index(text.len() + 1);
            while !text.is_char_boundary(at) {
                at -= 1;
            }
            let end = |len: usize| {
                let mut end = (at + len).min(text.len());
                while !text.is_char_boundary(end) {
                    end += 1;
                }
                end
            };
            match edit {
                Edit::Delete(len) => drop(text.drain(at..end(len))),
                Edit::Insert(fragment) => text.insert_str(at, fragment),
                Edit::Duplicate(len) => {
                    let copy = text[at..end(len)].to_owned();
                    text.insert_str(at, &copy);
                }
            }
        }
        text
    })
}

#[test]
fn the_seeds_keep_the_invariants() {
    for seed in SEEDS {
        nomi_syntax::check_invariants(seed);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn edited_seeds_keep_the_invariants(text in edited_seed()) {
        nomi_syntax::check_invariants(&text);
    }

    #[test]
    fn token_soups_keep_the_invariants(text in soup()) {
        nomi_syntax::check_invariants(&text);
    }
}
