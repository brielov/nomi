use nomi_base::Compat;

use crate::{
    LexErrorKind, SyntaxErrorKind as E, parse_declarations, parse_expressions, parse_module,
};

use super::{Source, Tool, validate};

const ELM: Tool = Tool::Elm(Source::Application, Compat::LATEST);

fn kinds(parse: &crate::Parse, tool: Tool) -> Vec<E> {
    validate(parse, tool).into_iter().map(|e| e.kind).collect()
}

#[test]
fn code_block_expressions_may_start_with_anything() {
    let parse = parse_expressions("f x\ny\n[ g ]");
    assert_eq!(parse.errors(), []);
    assert_eq!(kinds(&parse, Tool::ElmFormat), []);
}

#[test]
fn code_blocks_end_with_their_last_entry() {
    for text in [
        "a = 1\n",
        "a = 1 -- note",
        "a = 1\n-- note",
        "a = 1 {- note -}",
    ] {
        let parse = parse_declarations(text);
        let errors = validate(&parse, Tool::ElmFormat);
        assert_eq!(
            errors.iter().map(|e| e.kind).collect::<Vec<_>>(),
            [E::TrailingText],
            "{text:?}"
        );
        assert_eq!(u32::from(errors[0].range.start()), 5);
    }
    for text in ["", "-- note\na = 1", "{- note -}\na = 1\nb = 2"] {
        assert_eq!(
            kinds(&parse_declarations(text), Tool::ElmFormat),
            [],
            "{text:?}"
        );
    }
}

#[test]
fn a_code_block_expression_keeps_the_comment_ending_its_line() {
    assert_eq!(
        kinds(&parse_expressions("f x -- note"), Tool::ElmFormat),
        []
    );
    assert_eq!(
        kinds(&parse_expressions("f x -- note\n"), Tool::ElmFormat),
        [E::TrailingText]
    );
    assert_eq!(
        kinds(&parse_expressions("f x\n-- note"), Tool::ElmFormat),
        [E::TrailingText]
    );
    assert_eq!(
        kinds(&parse_expressions("f\n  x -- note"), Tool::ElmFormat),
        [E::TrailingText]
    );
}

#[test]
fn a_code_block_of_expressions_does_not_open_with_a_block_comment() {
    assert_eq!(
        kinds(&parse_expressions("{- note -}\nf x"), Tool::ElmFormat),
        [E::LeadingBlockComment]
    );
    for text in [
        " {- note -}\nf x",
        "\n{- note -}\nf x",
        "-- note\n{- note -}\nf x",
    ] {
        assert_eq!(
            kinds(&parse_expressions(text), Tool::ElmFormat),
            [],
            "{text:?}"
        );
    }
    let parse = parse_declarations("{- note -}\na = 1");
    assert_eq!(kinds(&parse, Tool::ElmFormat), []);
}

#[test]
fn a_code_block_variant_keeps_the_comment_ending_its_line() {
    for text in ["type T = A | B -- note", "type T\n    = A\n    | B -- note"] {
        assert_eq!(
            kinds(&parse_declarations(text), Tool::ElmFormat),
            [],
            "{text:?}"
        );
    }
    for text in [
        "type T = A | B\n    Int -- note",
        "type alias T = Int -- note",
    ] {
        let parse = parse_declarations(text);
        assert_eq!(
            kinds(&parse, Tool::ElmFormat),
            [E::TrailingText],
            "{text:?}"
        );
    }
}

#[test]
fn the_first_expression_may_start_with_anything() {
    let parse = parse_expressions("(f x)");
    assert_eq!(kinds(&parse, Tool::ElmFormat), []);
}

#[test]
fn a_comment_before_an_expression_is_not_its_start() {
    let parse = parse_expressions("f\n-- note\ng");
    assert_eq!(kinds(&parse, Tool::ElmFormat), []);
}

#[test]
fn declaration_fragments_follow_the_same_rules() {
    let parse = parse_declarations("( a, b ) = pair");
    assert_eq!(kinds(&parse, ELM), [E::DestructureDeclaration]);
    assert_eq!(kinds(&parse, Tool::ElmFormat), []);
}

#[test]
fn ranges_leave_out_leading_comments() {
    let text = "module M exposing (..)\n\n-- note\ninfix left 6 (+) = add\n\nadd = a\n";
    let parse = parse_module(text);
    let errors = validate(&parse, ELM);
    assert_eq!(errors.len(), 1);
    let range = errors[0].range;
    assert_eq!(
        &text[usize::from(range.start())..usize::from(range.end())],
        "infix left 6 (+) = add"
    );
}

#[test]
fn broken_trees_validate_without_panicking() {
    for text in [
        "module",
        "port module exposing",
        "effect module M where { = } exposing",
        "x : Int\n",
        "x =\n  { a : }\n",
        "f (\n",
        "type alias A = { x = }\n",
    ] {
        let parse = parse_module(text);
        for tool in [
            ELM,
            Tool::Elm(Source::Package, Compat::LATEST),
            Tool::Elm(Source::Kernel, Compat::LATEST),
            Tool::ElmFormat,
        ] {
            validate(&parse, tool);
        }
    }
}

#[test]
fn columns_wrap_in_16_bits_for_0_19_1() {
    // A list item at the 0-based `column`, after a long string; its
    // comma is two columns left.
    let module = |column: usize| {
        let padding = "a".repeat(column - 10);
        format!("module M exposing (x)\n\nx =\n    [ \"{padding}\", 1\n    ]\n")
    };
    let elm = |compat| Tool::Elm(Source::Application, compat);
    for (column, wraps) in [(65533, false), (65535, true), (65536, true), (65539, false)] {
        let parse = parse_module(&module(column));
        assert_eq!(parse.errors(), [], "column {column}");
        let expected: &[E] = if wraps { &[E::ExpectedIndent] } else { &[] };
        assert_eq!(
            kinds(&parse, elm(Compat::V0_19_1)),
            expected,
            "column {column}"
        );
        for compat in [Compat::V0_19_2, Compat::V0_19_3] {
            assert_eq!(kinds(&parse, elm(compat)), [], "column {column}");
        }
    }
}

#[test]
fn escapes_end_with_the_literal() {
    // A backslash ending an unterminated literal, and one escaping a
    // character of more than one byte.
    for text in ["x = \"a\\", "x = '\\", "x = \"\\é\\x41\""] {
        let parse = parse_module(text);
        validate(&parse, ELM);
        validate(&parse, Tool::ElmFormat);
    }
    // The lexer reports `\é`; validation, the `\x` after it.
    let text = "module M exposing (x)\n\n\nx =\n    \"\\é\\x41\"\n";
    let errors = validate(&parse_module(text), ELM);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].kind, E::Lexical(LexErrorKind::UnknownEscape));
    assert_eq!(&text[errors[0].range], "\\x");
}
