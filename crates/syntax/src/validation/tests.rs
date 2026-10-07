use crate::{SyntaxErrorKind as E, parse_declarations, parse_expressions, parse_module};

use super::{Source, Tool, validate};

const ELM: Tool = Tool::Elm(Source::Application);

fn kinds(parse: &crate::Parse, tool: Tool) -> Vec<E> {
    validate(parse, tool).into_iter().map(|e| e.kind).collect()
}

#[test]
fn code_blocks_continue_from_letters() {
    let parse = parse_expressions("f x\ny\n[ g ]\n");
    assert_eq!(parse.errors(), []);
    let errors = validate(&parse, Tool::ElmFormat);
    assert_eq!(
        errors.iter().map(|e| e.kind).collect::<Vec<_>>(),
        [E::ExpectedLetterStart]
    );
    assert_eq!(u32::from(errors[0].range.start()), 6);
}

#[test]
fn the_first_expression_may_start_with_anything() {
    let parse = parse_expressions("(f x)\n");
    assert_eq!(kinds(&parse, Tool::ElmFormat), []);
}

#[test]
fn a_comment_before_an_expression_is_not_its_start() {
    let parse = parse_expressions("f\n-- note\ng\n");
    assert_eq!(kinds(&parse, Tool::ElmFormat), []);
}

#[test]
fn declaration_fragments_follow_the_same_rules() {
    let parse = parse_declarations("( a, b ) = pair\n");
    assert_eq!(kinds(&parse, ELM), [E::DestructureDeclaration]);
    assert_eq!(kinds(&parse, Tool::ElmFormat), []);
}

#[test]
fn ranges_leave_out_leading_comments() {
    let text = "module M exposing (..)\n\n-- note\ninfix left 6 (+) = add\n";
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
            Tool::Elm(Source::Package),
            Tool::Elm(Source::Kernel),
            Tool::ElmFormat,
        ] {
            validate(&parse, tool);
        }
    }
}
