use nomi_syntax::{LexErrorKind, Lexed, SyntaxKind, lex};
use proptest::prelude::*;

fn texts<'a>(text: &'a str, lexed: &Lexed) -> Vec<&'a str> {
    lexed
        .tokens()
        .iter()
        .map(|token| &text[token.range])
        .collect()
}

/// The non-whitespace tokens, with their texts.
fn significant(text: &str) -> Vec<(SyntaxKind, &str)> {
    lex(text)
        .tokens()
        .iter()
        .filter(|token| token.kind != SyntaxKind::Whitespace)
        .map(|token| (token.kind, &text[token.range]))
        .collect()
}

fn kinds(text: &str) -> Vec<SyntaxKind> {
    lex(text).tokens().iter().map(|token| token.kind).collect()
}

fn errors(text: &str) -> Vec<LexErrorKind> {
    lex(text).errors().iter().map(|error| error.kind).collect()
}

/// Every byte belongs to exactly one nonempty token, in order.
fn assert_lossless(text: &str) {
    let lexed = lex(text);
    let mut end = 0;
    for token in lexed.tokens() {
        assert_eq!(usize::from(token.range.start()), end, "{text:?}");
        assert!(!token.range.is_empty(), "{text:?}");
        end = token.range.end().into();
    }
    assert_eq!(end, text.len(), "{text:?}");
    assert_eq!(texts(text, &lexed).concat(), text);
}

#[test]
fn identifiers_use_the_reference_compilers_unicode_version() {
    // U+0295 changed category after Unicode 15.1; U+1C89 and U+1C8A were
    // unassigned there. A newer table changes what Elm accepts.
    assert!(errors("\u{0295}").is_empty());
    assert_eq!(kinds("\u{0295}"), [SyntaxKind::LowerIdent]);
    for text in ["\u{1C89}", "\u{1C8A}"] {
        assert_eq!(errors(text), [LexErrorKind::InvalidToken]);
        assert_lossless(text);
    }
    // CJK Extension I is assigned in 15.1 and may continue an identifier.
    assert_eq!(
        significant("x\u{2EBF0}"),
        [(SyntaxKind::LowerIdent, "x\u{2EBF0}")]
    );
}

#[test]
fn nul_in_comments_is_reported_without_losing_source_bytes() {
    let text = "-- a\0\n{- b\0 -}\n{-| c\0 -}\n";
    assert_lossless(text);
    assert_eq!(errors(text), [LexErrorKind::UnexpectedEncoding; 3]);
}

#[test]
fn a_whole_module_is_lossless_and_clean() {
    let text = "module Café exposing (..)\r\n\
                {- outer {- nested -} -}\n\
                {-| docs -}\n\
                answer = -12.5e+2\n\
                text = \"Elm\\nλ\"\n\
                multiline = \"\"\"one\n\
                two\"\"\"\n\
                letter = '\\u{03BB}'\n\
                shader = [glsl|uniform vec2 size;|]\n\
                pipeline = value |> Result.map identity\n";
    assert_lossless(text);
    assert_eq!(errors(text), []);
}

#[test]
fn qualification_and_negation_remain_parser_decisions() {
    assert_eq!(
        significant("Html.Attributes.class -42"),
        [
            (SyntaxKind::UpperIdent, "Html"),
            (SyntaxKind::Dot, "."),
            (SyntaxKind::UpperIdent, "Attributes"),
            (SyntaxKind::Dot, "."),
            (SyntaxKind::LowerIdent, "class"),
            (SyntaxKind::Minus, "-"),
            (SyntaxKind::Int, "42"),
        ]
    );
}

#[test]
fn only_reserved_words_are_lexical_keywords() {
    let text = "if then else case of let in type module where import exposing as port \
                alias infix left right non effect command subscription";
    let keywords: Vec<_> = significant(text)
        .into_iter()
        .map(|(kind, _)| kind)
        .collect();
    use SyntaxKind::*;
    assert_eq!(
        keywords,
        [
            IfKw, ThenKw, ElseKw, CaseKw, OfKw, LetKw, InKw, TypeKw, ModuleKw, WhereKw, ImportKw,
            ExposingKw, AsKw, PortKw, LowerIdent, LowerIdent, LowerIdent, LowerIdent, LowerIdent,
            LowerIdent, LowerIdent, LowerIdent,
        ]
    );
}

#[test]
fn operators_with_kinds_of_their_own() {
    use SyntaxKind::*;
    assert_eq!(
        significant(". .. | = : :: -> - + |> <| =- --x\n.."),
        [
            (Dot, "."),
            (DotDot, ".."),
            (Pipe, "|"),
            (Eq, "="),
            (Colon, ":"),
            (ColonColon, "::"),
            (Arrow, "->"),
            (Minus, "-"),
            (Operator, "+"),
            (Operator, "|>"),
            (Operator, "<|"),
            (Operator, "=-"),
            (LineComment, "--x"),
            (Newline, "\n"),
            (DotDot, ".."),
        ]
    );
    assert_eq!(
        significant("(\\_,[]{})"),
        [
            (LParen, "("),
            (Backslash, "\\"),
            (Underscore, "_"),
            (Comma, ","),
            (LBracket, "["),
            (RBracket, "]"),
            (LBrace, "{"),
            (RBrace, "}"),
            (RParen, ")"),
        ]
    );
}

#[test]
fn identifier_classes_follow_unicode_general_categories() {
    use SyntaxKind::*;
    let text = "lower2 Δvalue ǅvalue value名 valueª valueʰ ªvalue Ⅷvalue valueⅧ valueֿ";
    assert_eq!(
        significant(text),
        [
            (LowerIdent, "lower2"),
            (UpperIdent, "Δvalue"),
            (UpperIdent, "ǅvalue"),
            (LowerIdent, "value名"),
            (LowerIdent, "valueª"),
            (LowerIdent, "valueʰ"),
            (ErrorToken, "ª"),
            (LowerIdent, "value"),
            (ErrorToken, "Ⅷ"),
            (LowerIdent, "value"),
            (LowerIdent, "value"),
            (ErrorToken, "Ⅷ"),
            (LowerIdent, "value"),
            (ErrorToken, "ֿ"),
        ]
    );
    assert_eq!(errors(text), [LexErrorKind::InvalidToken; 4]);
    assert_eq!(
        significant("snake_case Upper_1"),
        [(LowerIdent, "snake_case"), (UpperIdent, "Upper_1")]
    );
}

#[test]
fn comments_are_lossless_and_nested() {
    let text = "-- line\r\n{- a {- b -} c -}{-| documentation -}";
    assert_eq!(
        kinds(text),
        [
            SyntaxKind::LineComment,
            SyntaxKind::Newline,
            SyntaxKind::BlockComment,
            SyntaxKind::DocComment,
        ]
    );
    assert_lossless(text);
    assert_eq!(errors(text), []);
}

#[test]
fn a_comment_opener_needs_a_byte_after_it() {
    assert_eq!(kinds("{-"), [SyntaxKind::LBrace, SyntaxKind::Minus]);
    assert_eq!(kinds("{--}"), [SyntaxKind::BlockComment]);
}

#[test]
fn lines_end_at_line_feeds() {
    use SyntaxKind::*;
    assert_eq!(
        significant("a\r\nb\nc\rd \r \r\n"),
        [
            (LowerIdent, "a"),
            (Newline, "\r\n"),
            (LowerIdent, "b"),
            (Newline, "\n"),
            (LowerIdent, "c"),
            (LowerIdent, "d"),
            (Newline, "\r\n"),
        ]
    );
    // A bare carriage return doesn't end a line comment.
    assert_eq!(
        kinds("-- a\rb\r\n"),
        [SyntaxKind::LineComment, SyntaxKind::Newline]
    );
    assert_eq!(
        texts("c\rd \r \r\n", &lex("c\rd \r \r\n")),
        ["c", "\r", "d", " \r ", "\r\n"]
    );
}

#[test]
fn malformed_tokens_are_preserved_with_precise_errors() {
    // The tab in the comment is validation's.
    let text = "\t{- open\t\n\"open\n'' 01 0x 1e+ [glsl|open §";
    assert_eq!(
        errors(text),
        [LexErrorKind::Tab, LexErrorKind::UnterminatedBlockComment]
    );
    assert_lossless(text);
}

#[test]
fn independent_malformed_tokens_report_without_losing_input() {
    let text = "\"open\n'' 01 0x 1e+ [glsl|open";
    assert_eq!(
        errors(text),
        [
            LexErrorKind::UnterminatedString,
            LexErrorKind::CharacterLength { characters: 0 },
            LexErrorKind::LeadingZero,
            LexErrorKind::InvalidHexDigit,
            LexErrorKind::MissingExponentDigits,
            LexErrorKind::UnterminatedShaderBlock,
        ]
    );
    assert_lossless(text);
    // Malformed literals keep their kinds for the parser.
    use SyntaxKind::*;
    assert_eq!(
        significant(text),
        [
            (String, "\"open"),
            (Newline, "\n"),
            (Char, "''"),
            (Int, "0"),
            (Int, "1"),
            (Int, "0x"),
            (Float, "1e+"),
            (Glsl, "[glsl|open"),
        ]
    );
}

#[test]
fn unterminated_literals_and_odd_characters() {
    assert_eq!(
        errors("'ab' 'a\n'\\"),
        [
            LexErrorKind::CharacterLength { characters: 2 },
            LexErrorKind::UnterminatedCharacter,
            LexErrorKind::UnterminatedCharacter,
        ]
    );
    assert_eq!(
        errors("\"\"\"a\nb"),
        [LexErrorKind::UnterminatedMultilineString]
    );
    assert_eq!(errors("\"\\q\" '\\n'"), [LexErrorKind::UnknownEscape]);
    assert_eq!(errors(r#""\n\r\t\"\'\\" '\t'"#), []);
    assert_eq!(errors("\"a\\"), [LexErrorKind::UnterminatedString]);
    assert_eq!(errors("{- {- -}"), [LexErrorKind::UnterminatedBlockComment]);
    assert_eq!(kinds("{-| x"), [SyntaxKind::DocComment]);
    assert_eq!(errors("#"), [LexErrorKind::InvalidToken]);
    assert_eq!(
        kinds("#`"),
        [SyntaxKind::ErrorToken, SyntaxKind::ErrorToken]
    );
}

#[test]
fn error_ranges_point_at_the_problem() {
    let at = |text: &str| -> Vec<(usize, usize)> {
        lex(text)
            .errors()
            .iter()
            .map(|e| (e.range.start().into(), e.range.end().into()))
            .collect()
    };
    assert_eq!(at("x \"ab\ny"), [(2, 5)]);
    assert_eq!(at("1e"), [(2, 2)]);
    assert_eq!(at("01"), [(1, 1)]);
    assert_eq!(at("12x"), [(2, 2)]);
    assert_eq!(at("\"\\u{1}\""), [(1, 6)]);
    assert_eq!(at(" \t"), [(1, 2)]);
    let lexed = lex("12.x");
    assert_eq!(
        lexed.errors()[0].kind,
        LexErrorKind::MissingFractionDigits {
            integer: lexed.tokens()[0].range
        }
    );
    assert_eq!(at("12.x"), [(2, 2)]);
}

#[test]
fn number_boundaries_follow_the_reference_chompers() {
    let text = "0e3 1e2name 1.2name";
    assert_eq!(
        errors(text),
        [
            LexErrorKind::NumberFollowedByIdentifier,
            LexErrorKind::NumberFollowedByIdentifier,
        ]
    );
    use SyntaxKind::*;
    assert_eq!(
        significant(text),
        [
            (Int, "0"),
            (LowerIdent, "e3"),
            (Float, "1e2"),
            (LowerIdent, "name"),
            (Float, "1.2"),
            (LowerIdent, "name"),
        ]
    );
    assert_eq!(
        significant("0x1F 0.5 7E-3 1.5e10 0xAg 0.x"),
        [
            (Int, "0x1F"),
            (Float, "0.5"),
            (Float, "7E-3"),
            (Float, "1.5e10"),
            (Int, "0xA"),
            (LowerIdent, "g"),
            (Int, "0"),
            (Dot, "."),
            (LowerIdent, "x"),
        ]
    );
    assert_eq!(
        errors("0xAg 0.x"),
        [
            LexErrorKind::InvalidHexDigit,
            LexErrorKind::MissingFractionDigits {
                integer: nomi_base::TextRange::new(5.into(), 6.into())
            }
        ]
    );
}

#[test]
fn unicode_escape_failures_retain_the_reason_and_literal_text() {
    for (literal, expected) in [
        (r#""\u0041""#, LexErrorKind::InvalidUnicodeEscapeFormat),
        (r#""\u{12Z}""#, LexErrorKind::InvalidUnicodeEscapeFormat),
        (r#""\u{}""#, LexErrorKind::InvalidUnicodeCodePoint),
        (r#""\u{110000}""#, LexErrorKind::InvalidUnicodeCodePoint),
        (
            r#""\u{FFFFFFFFFFFFFFFFFFFFFFFF}""#,
            LexErrorKind::InvalidUnicodeCodePoint,
        ),
        (
            r#""\u{00}""#,
            LexErrorKind::InvalidUnicodeEscapeLength {
                digits: 2,
                value: 0,
            },
        ),
        (
            r#""\u{0ab}""#,
            LexErrorKind::InvalidUnicodeEscapeLength {
                digits: 3,
                value: 0xab,
            },
        ),
        (
            r#""\u{0000000}""#,
            LexErrorKind::InvalidUnicodeEscapeLength {
                digits: 7,
                value: 0,
            },
        ),
    ] {
        assert_lossless(literal);
        assert_eq!(errors(literal), [expected], "{literal}");
        assert_eq!(kinds(literal), [SyntaxKind::String], "{literal}");
    }
    // The official parser accepts surrogate code points as well as scalar
    // values.
    for literal in [r#""\u{0000}""#, r#""\u{D800}""#, r#""\u{10FFFF}""#] {
        assert_eq!(errors(literal), [], "{literal}");
        assert_eq!(kinds(literal), [SyntaxKind::String], "{literal}");
    }
}

/// Text built from fragments that exercise every lexer path, so random
/// inputs reach the interesting states more often than arbitrary strings.
fn elmish() -> impl Strategy<Value = std::string::String> {
    let fragment = prop_oneof![
        Just("{-"),
        Just("-}"),
        Just("{-|"),
        Just("--"),
        Just("\""),
        Just("\"\"\""),
        Just("'"),
        Just("\\"),
        Just("\\u{"),
        Just("}"),
        Just("[glsl|"),
        Just("|]"),
        Just("0x"),
        Just("0"),
        Just("1.5e"),
        Just("."),
        Just("\n"),
        Just("\r"),
        Just("\r\n"),
        Just("\t"),
        Just("\0"),
        Just(" "),
        Just("if"),
        Just("x"),
        Just("Δ"),
        Just("ª"),
        Just("名"),
        Just("=-"),
        Just("|>"),
    ]
    .prop_map(str::to_owned);
    let any = any::<char>().prop_map(std::string::String::from);
    prop::collection::vec(prop_oneof![3 => fragment, 1 => any], 0..40)
        .prop_map(|parts| parts.concat())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2_000))]

    #[test]
    fn tokens_concatenate_back_to_the_input(text in elmish()) {
        assert_lossless(&text);
        for error in lex(&text).errors() {
            prop_assert!(usize::from(error.range.end()) <= text.len());
            prop_assert!(text.is_char_boundary(error.range.start().into()));
            prop_assert!(text.is_char_boundary(error.range.end().into()));
        }
    }

    #[test]
    fn arbitrary_strings_are_lossless(text in any::<std::string::String>()) {
        assert_lossless(&text);
    }
}
