use super::super::c;
use crate::tree::{Comment, ForceMultiline, Lower, Pair, Sequence, Type, TypeConstructor, Upper};

fn lower(text: &str) -> Type {
    crate::lower::type_fragment(text)
}

fn block(text: &str) -> Comment {
    Comment::Block(vec![text.to_owned()])
}

fn var(name: &str) -> Type {
    Type::Variable(Lower(name.to_owned()))
}

fn named(namespace: &[&str], name: &str, args: Vec<Type>) -> Type {
    let namespace = namespace.iter().map(|n| Upper((*n).to_owned())).collect();
    let args = args.into_iter().map(|a| c(Vec::new(), a)).collect();
    Type::Construction(
        TypeConstructor::Named((namespace, Upper(name.to_owned()))),
        args,
        ForceMultiline(false),
    )
}

fn eol(text: &str) -> Option<String> {
    Some(text.to_owned())
}

#[test]
fn arrows_keep_the_comments_ending_each_line() {
    let text = "a -- one\n -> {- p -} b -- two\n -> (c -> d) {- q -} -> e -- three";
    let Type::Function {
        first,
        rest,
        force_multiline,
    } = lower(text)
    else {
        panic!()
    };
    assert_eq!(*first, c(eol(" one"), var("a")));
    let function = Type::Function {
        first: Box::new(c(None, var("c"))),
        rest: Sequence(vec![c((vec![], vec![], None), var("d"))]),
        force_multiline: ForceMultiline(false),
    };
    assert_eq!(
        rest,
        Sequence(vec![
            c((vec![], vec![block("p")], eol(" two")), var("b")),
            c((vec![], vec![], None), function),
            c((vec![block("q")], vec![], eol(" three")), var("e")),
        ])
    );
    assert_eq!(force_multiline, ForceMultiline(true));
}

#[test]
fn parentheses_without_comments_go() {
    assert_eq!(lower("((a))"), var("a"));
    assert_eq!(
        lower("( {- x -} a -- e\n)"),
        Type::Parens(Box::new(c(
            (vec![block("x")], vec![Comment::Line(" e".to_owned())]),
            var("a")
        )))
    );
    assert_eq!(lower("( {- u -} )"), Type::Unit(vec![block("u")]));
}

#[test]
fn tuples() {
    let items: Vec<_> = vec![
        c((vec![], vec![], eol(" e")), var("a")),
        c((vec![block("x")], vec![block("y")], None), var("b")),
    ];
    assert_eq!(
        lower("( a -- e\n, {- x -} b {- y -} )"),
        Type::Tuple(items, ForceMultiline(true))
    );
}

#[test]
fn constructors_and_their_arguments() {
    let Type::Construction(ctor, args, ForceMultiline(false)) =
        lower("Dict.Dict {- k -} String (List a)")
    else {
        panic!()
    };
    assert_eq!(
        ctor,
        TypeConstructor::Named((vec![Upper("Dict".to_owned())], Upper("Dict".to_owned())))
    );
    assert_eq!(
        args,
        [
            c(vec![block("k")], named(&[], "String", vec![])),
            c(vec![], named(&[], "List", vec![var("a")])),
        ]
    );
    let Type::Construction(TypeConstructor::Tuple(3), args, ForceMultiline(true)) =
        lower("(,,) a\n b c")
    else {
        panic!()
    };
    assert_eq!(args.len(), 3);
}

#[test]
fn records() {
    let text =
        "{ {- b -} r {- p -} | {- x -} a : Int -- e\n , {- y -} b {- k -} = {- v -} B {- t -} }";
    let pair = |key: &str, after_key, before_value, value| Pair {
        key: c(after_key, Lower(key.to_owned())),
        value: c(before_value, value),
        force_multiline: ForceMultiline(false),
    };
    assert_eq!(
        lower(text),
        Type::Record {
            base: Some(c(
                (vec![block("b")], vec![block("p")]),
                Lower("r".to_owned())
            )),
            fields: Sequence(vec![
                c(
                    (vec![], vec![block("x")], eol(" e")),
                    pair("a", vec![], vec![], named(&[], "Int", vec![]))
                ),
                c(
                    (vec![], vec![block("y")], None),
                    pair(
                        "b",
                        vec![block("k")],
                        vec![block("v")],
                        named(&[], "B", vec![])
                    )
                ),
            ]),
            trailing: vec![block("t")],
            force_multiline: ForceMultiline(true),
        }
    );
    assert_eq!(
        lower("{ {- c -} }"),
        Type::Record {
            base: None,
            fields: Sequence(Vec::new()),
            trailing: vec![block("c")],
            force_multiline: ForceMultiline(false),
        }
    );
}

#[test]
fn a_field_over_lines_ends_no_line_comment() {
    let Type::Record {
        fields, trailing, ..
    } = lower("{ a :\n Int -- e\n }")
    else {
        panic!()
    };
    assert_eq!(fields.0[0].comments.2, None);
    assert!(fields.0[0].value.force_multiline.0);
    assert_eq!(trailing, [Comment::Line(" e".to_owned())]);
}
