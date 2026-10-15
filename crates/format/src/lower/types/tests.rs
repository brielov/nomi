use super::super::c;
use crate::test_support::{block, line, lower as name, named, separated, upper_ref};
use crate::tree::{Around, AroundEol, ForceMultiline, Pair, Sequence, Type, TypeConstructor};

fn lower(text: &str) -> Type {
    crate::lower::type_fragment(text)
}

fn var(name: &str) -> Type {
    Type::Variable(self::name(name))
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
    assert_eq!(*first, c(Some(" one".to_owned()), var("a")));
    let function = Type::Function {
        first: Box::new(c(None, var("c"))),
        rest: Sequence(vec![c(separated(vec![], vec![], None), var("d"))]),
        force_multiline: ForceMultiline(false),
    };
    assert_eq!(
        rest,
        Sequence(vec![
            c(separated(vec![], vec![block("p")], Some(" two")), var("b")),
            c(separated(vec![], vec![], None), function),
            c(
                separated(vec![block("q")], vec![], Some(" three")),
                var("e")
            ),
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
            Around::new(vec![block("x")], vec![line(" e")]),
            var("a")
        )))
    );
    assert_eq!(lower("( {- u -} )"), Type::Unit(vec![block("u")]));
}

#[test]
fn tuples() {
    let items = vec![
        c(
            AroundEol {
                before: vec![],
                after: vec![],
                eol: Some(" e".to_owned()),
            },
            var("a"),
        ),
        c(
            AroundEol {
                before: vec![block("x")],
                after: vec![block("y")],
                eol: None,
            },
            var("b"),
        ),
    ];
    assert_eq!(
        lower("( a -- e\n, {- x -} b {- y -} )"),
        Type::Tuple(items, ForceMultiline(true))
    );
}

#[test]
fn constructors_and_their_arguments() {
    let Type::Construction {
        ctor,
        args,
        force_multiline: ForceMultiline(false),
    } = lower("Dict.Dict {- k -} String (List a)")
    else {
        panic!()
    };
    assert_eq!(ctor, TypeConstructor::Named(upper_ref(&["Dict"], "Dict")));
    assert_eq!(
        args,
        [
            c(vec![block("k")], named(&[], "String", vec![])),
            c(vec![], named(&[], "List", vec![var("a")])),
        ]
    );
    let Type::Construction {
        ctor: TypeConstructor::Tuple(3),
        args,
        force_multiline: ForceMultiline(true),
    } = lower("(,,) a\n b c")
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
        key: c(after_key, name(key)),
        value: c(before_value, value),
        force_multiline: ForceMultiline(false),
    };
    assert_eq!(
        lower(text),
        Type::Record {
            base: Some(c(
                Around::new(vec![block("b")], vec![block("p")]),
                name("r")
            )),
            fields: Sequence(vec![
                c(
                    separated(vec![], vec![block("x")], Some(" e")),
                    pair("a", vec![], vec![], named(&[], "Int", vec![]))
                ),
                c(
                    separated(vec![], vec![block("y")], None),
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
    assert_eq!(fields.0[0].comments.eol, None);
    assert!(fields.0[0].value.force_multiline.0);
    assert_eq!(trailing, [line(" e")]);
}
