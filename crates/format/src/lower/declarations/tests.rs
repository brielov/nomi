use crate::test_support::{block, line, lower, lowered, named, upper};
use crate::tree::{
    AnnotatedName, Around, AroundEol, Assoc, BeforeEol, Comment, CommonDeclaration, Declaration,
    Docs, Expr, IntRepr, LiteralValue, NameWithArgs, OpenCommentedList, Pattern, Symbol,
    TopLevelStructure, Type, c,
};

type Body = Vec<TopLevelStructure<Declaration>>;

/// The declarations of `text`, after a header and an import, without
/// which the first doc comment would be the module's.
fn body(text: &str) -> Body {
    lowered(&format!(
        "port module A exposing (..)\n\nimport B\n\n\n{text}"
    ))
    .body
}

fn comment(c: Comment) -> TopLevelStructure<Declaration> {
    TopLevelStructure::BodyComment(c)
}

fn entry(d: Declaration) -> TopLevelStructure<Declaration> {
    TopLevelStructure::Entry(d)
}

fn simple<A>(name: &str) -> NameWithArgs<A> {
    NameWithArgs {
        name: upper(name),
        args: vec![],
    }
}

/// One comment before, and one after.
fn around(before: Comment, after: Comment) -> Around {
    Around::new(vec![before], vec![after])
}

#[test]
fn values_split_into_their_parts() {
    let text = "-- a\n{-| Doc. -}\n-- b\nx : Int\n-- c\nx = 1\n-- d\n";
    assert_eq!(
        body(text),
        [
            comment(line(" a")),
            TopLevelStructure::DocComment(Docs {
                blocks: crate::markdown::parse("Doc.", &mut |_, _| None),
                source: "{-| Doc. -}".to_owned(),
            }),
            comment(line(" b")),
            entry(Declaration::Common(CommonDeclaration::TypeAnnotation {
                name: c(vec![], AnnotatedName::Var(lower("x"))),
                ty: c(vec![], named(&[], "Int", vec![])),
            })),
            comment(line(" c")),
            entry(Declaration::Common(CommonDeclaration::Definition {
                name: Pattern::Var(lower("x")),
                args: vec![],
                before_body: vec![],
                body: Expr::Literal(LiteralValue::Int(1, IntRepr::Decimal)),
            })),
            comment(line(" d")),
        ]
    );
}

#[test]
fn custom_types_keep_comments_around_each_variant() {
    let text = "type {- t -} T {- p -} a {- e -} = {- f -} A a -- one\n  | {- g -} B\n";
    let [
        TopLevelStructure::Entry(Declaration::Datatype {
            name_with_args,
            variants,
        }),
    ] = &body(text)[..]
    else {
        panic!()
    };
    assert_eq!(
        *name_with_args,
        c(
            around(block("t"), block("e")),
            NameWithArgs {
                name: upper("T"),
                args: vec![c(vec![block("p")], lower("a"))],
            }
        )
    );
    assert_eq!(
        *variants,
        OpenCommentedList {
            items: vec![c(
                AroundEol {
                    before: vec![block("f")],
                    after: vec![],
                    eol: Some(" one".to_owned()),
                },
                NameWithArgs {
                    name: upper("A"),
                    args: vec![c(vec![], Type::Variable(lower("a")))],
                }
            )],
            last: c(
                BeforeEol {
                    before: vec![block("g")],
                    eol: None,
                },
                simple("B")
            ),
        }
    );
    // The `--` comment ending the last variant's line is its own; any
    // other after it is between declarations.
    let Declaration::Datatype { variants, .. } = datatype("type T = A -- e\n") else {
        panic!()
    };
    assert_eq!(variants.last.comments.eol, Some(" e".to_owned()));
    assert_eq!(body("type T = A {- h -}\n")[1], comment(block("h")));
}

fn datatype(text: &str) -> Declaration {
    let [TopLevelStructure::Entry(d)] = &body(text)[..] else {
        panic!()
    };
    d.clone()
}

#[test]
fn aliases_and_ports() {
    assert_eq!(
        body("type {- a -} alias {- b -} T {- c -} = {- d -} Int\n"),
        [entry(Declaration::TypeAlias {
            before_alias: vec![block("a")],
            name_with_args: c(around(block("b"), block("c")), simple("T")),
            ty: c(vec![block("d")], named(&[], "Int", vec![])),
        })]
    );
    assert_eq!(
        body("port {- a -} out {- b -} : {- c -} Int\n"),
        [entry(Declaration::PortAnnotation {
            name: c(around(block("a"), block("b")), lower("out")),
            before_type: vec![block("c")],
            ty: named(&[], "Int", vec![]),
        })]
    );
}

#[test]
fn fixities() {
    let text = "infix {- a -} left {- b -} 6 {- c -} (+) {- d -} = {- e -} add\n";
    let parse = nomi_syntax::parse_module(text);
    assert!(parse.errors().is_empty(), "{:?}", parse.errors());
    assert_eq!(
        super::super::module(&parse.syntax(), text).body,
        [entry(Declaration::Fixity {
            assoc: c(vec![block("a")], Assoc::Left),
            precedence: c(vec![block("b")], 6),
            op: c(around(block("c"), block("d")), Symbol("+".to_owned())),
            function: c(vec![block("e")], lower("add")),
        })]
    );
}
