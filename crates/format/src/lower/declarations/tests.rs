use nomi_syntax::{Tool, parse_module, validate};

use super::super::{c, module};
use crate::tree::{
    Assoc, Comment, CommonDeclaration, Declaration, Docs, Expr, ForceMultiline, IntRepr,
    LiteralValue, Lower, NameWithArgs, OpenCommentedList, Pattern, Ref, Symbol, TopLevelStructure,
    Type, TypeConstructor, Upper,
};

type Body = Vec<TopLevelStructure<Declaration>>;

/// The declarations of `text`, after a header and an import, without
/// which the first doc comment would be the module's.
fn body(text: &str) -> Body {
    let text = format!("port module A exposing (..)\n\nimport B\n\n\n{text}");
    let parse = parse_module(&text);
    assert!(parse.errors().is_empty(), "{:?}", parse.errors());
    assert!(validate(&parse, Tool::ElmFormat).is_empty());
    module(&parse.syntax(), &text).body
}

fn block(text: &str) -> Comment {
    Comment::Block(vec![text.to_owned()])
}

fn line(text: &str) -> Comment {
    Comment::Line(text.to_owned())
}

fn comment(c: Comment) -> TopLevelStructure<Declaration> {
    TopLevelStructure::BodyComment(c)
}

fn entry(d: Declaration) -> TopLevelStructure<Declaration> {
    TopLevelStructure::Entry(d)
}

fn lower(name: &str) -> Lower {
    Lower(name.to_owned())
}

fn upper(name: &str) -> Upper {
    Upper(name.to_owned())
}

fn named(name: &str) -> Type {
    Type::Construction(
        TypeConstructor::Named((vec![], upper(name))),
        vec![],
        ForceMultiline(false),
    )
}

fn simple<A>(name: &str) -> NameWithArgs<Upper, A> {
    NameWithArgs {
        name: upper(name),
        args: vec![],
    }
}

#[test]
fn values_split_into_their_parts() {
    let text = "-- a\n{-| Doc. -}\n-- b\nx : Int\n-- c\nx = 1\n-- d\n";
    assert_eq!(
        body(text),
        [
            comment(line(" a")),
            TopLevelStructure::DocComment(Docs("Doc.".to_owned())),
            comment(line(" b")),
            entry(Declaration::Common(CommonDeclaration::TypeAnnotation(
                c(vec![], Ref::Var((), lower("x"))),
                c(vec![], named("Int"))
            ))),
            comment(line(" c")),
            entry(Declaration::Common(CommonDeclaration::Definition(
                Pattern::Var(lower("x")),
                vec![],
                vec![],
                Expr::Literal(LiteralValue::Int(1, IntRepr::Decimal))
            ))),
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
            (vec![block("t")], vec![block("e")]),
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
                (vec![block("f")], vec![], Some(" one".to_owned())),
                NameWithArgs {
                    name: upper("A"),
                    args: vec![c(vec![], Type::Variable(lower("a")))],
                }
            )],
            last: c((vec![block("g")], None), simple("B")),
        }
    );
    // The `--` comment ending the last variant's line is its own; any
    // other after it is between declarations.
    let Declaration::Datatype { variants, .. } = datatype("type T = A -- e\n") else {
        panic!()
    };
    assert_eq!(variants.last.comments.1, Some(" e".to_owned()));
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
        [entry(Declaration::TypeAlias(
            vec![block("a")],
            c((vec![block("b")], vec![block("c")]), simple("T")),
            c(vec![block("d")], named("Int"))
        ))]
    );
    assert_eq!(
        body("port {- a -} out {- b -} : {- c -} Int\n"),
        [entry(Declaration::PortAnnotation(
            c((vec![block("a")], vec![block("b")]), lower("out")),
            vec![block("c")],
            named("Int")
        ))]
    );
}

#[test]
fn fixities() {
    let text = "infix {- a -} left {- b -} 6 {- c -} (+) {- d -} = {- e -} add\n";
    let parse = parse_module(text);
    assert!(parse.errors().is_empty(), "{:?}", parse.errors());
    assert_eq!(
        module(&parse.syntax(), text).body,
        [entry(Declaration::Fixity(
            c(vec![block("a")], Assoc::Left),
            c(vec![block("b")], 6),
            c((vec![block("c")], vec![block("d")]), Symbol("+".to_owned())),
            c(vec![block("e")], lower("add"))
        ))]
    );
}
