use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::parse_expr;

use super::super::cursor::Cursor;
use super::super::{VALID, c};
use super::expr;
use crate::tree::{
    ApplicationMultiline, BinopsClause, CaseBranch, Comment, CommonDeclaration, Expr,
    ForceMultiline, IfClause, IntRepr, LetDeclaration, LiteralValue, Lower, Multiline, Pair,
    Pattern, Ref, Sequence, Symbol, Type, TypeConstructor, UnaryOperator, Upper,
};

/// The expression `text` holds, which must place every comment.
fn lower(text: &str) -> Expr {
    let parse = parse_expr(text);
    assert!(parse.errors().is_empty(), "{:?}", parse.errors());
    let root = parse.syntax();
    let mut cx = Cursor::new(&root, text);
    let node = ast::ExprFragment::cast(root)
        .expect(VALID)
        .expr()
        .expect(VALID);
    let lowered = expr(&mut cx, &node);
    cx.finish();
    lowered
}

fn block(text: &str) -> Comment {
    Comment::Block(vec![text.to_owned()])
}

fn line(text: &str) -> Comment {
    Comment::Line(text.to_owned())
}

fn var(name: &str) -> Expr {
    Expr::Var(Ref::Var(Vec::new(), Lower(name.to_owned())))
}

fn int(value: i64) -> Expr {
    Expr::Literal(LiteralValue::Int(value, IntRepr::Decimal))
}

fn app(function: Expr, args: Vec<Expr>, multiline: ApplicationMultiline) -> Expr {
    let args = args.into_iter().map(|a| c(Vec::new(), a)).collect();
    Expr::App(Box::new(function), args, multiline)
}

const JOINED: ApplicationMultiline = ApplicationMultiline::JoinFirst(Multiline::JoinAll);

#[test]
fn names_and_literals() {
    assert_eq!(
        lower("List.map"),
        Expr::Var(Ref::Var(
            vec![Upper("List".to_owned())],
            Lower("map".to_owned())
        ))
    );
    assert_eq!(lower("True"), Expr::Literal(LiteralValue::Bool(true)));
    assert_eq!(
        lower("Basics.True"),
        Expr::Var(Ref::Tag(
            vec![Upper("Basics".to_owned())],
            Upper("True".to_owned())
        ))
    );
    assert_eq!(lower("-5"), int(-5));
    assert_eq!(
        lower("-x"),
        Expr::Unary(UnaryOperator::Negative, Box::new(var("x")))
    );
    assert_eq!(lower("(+)"), Expr::Var(Ref::Op(Symbol("+".to_owned()))));
    assert_eq!(lower("(,,)"), Expr::TupleFunction(3));
    assert_eq!(
        lower(".name"),
        Expr::AccessFunction(Lower("name".to_owned()))
    );
    assert_eq!(
        lower("a.b.c"),
        Expr::Access(
            Box::new(Expr::Access(Box::new(var("a")), Lower("b".to_owned()))),
            Lower("c".to_owned())
        )
    );
    assert_eq!(
        lower("[glsl|\r\nvoid main() {}|]"),
        Expr::GlShader("\nvoid main() {}".to_owned())
    );
}

#[test]
fn applications_break_as_the_source_does() {
    assert_eq!(
        lower("f a b"),
        app(var("f"), vec![var("a"), var("b")], JOINED)
    );
    assert_eq!(
        lower("f a\n b"),
        app(
            var("f"),
            vec![var("a"), var("b")],
            ApplicationMultiline::JoinFirst(Multiline::SplitAll)
        )
    );
    assert_eq!(
        lower("f\n a b"),
        app(
            var("f"),
            vec![var("a"), var("b")],
            ApplicationMultiline::SplitFirst
        )
    );
    assert_eq!(
        lower("f {- x -} a"),
        Expr::App(
            Box::new(var("f")),
            vec![c(vec![block("x")], var("a"))],
            JOINED
        )
    );
}

#[test]
fn operators_stay_flat() {
    assert_eq!(
        lower("a {- x -} + {- y -} b * c"),
        Expr::Binops(
            Box::new(var("a")),
            vec![
                BinopsClause {
                    before_op: vec![block("x")],
                    op: Ref::Op(Symbol("+".to_owned())),
                    after_op: vec![block("y")],
                    expr: var("b"),
                },
                BinopsClause {
                    before_op: vec![],
                    op: Ref::Op(Symbol("*".to_owned())),
                    after_op: vec![],
                    expr: var("c"),
                },
            ],
            false
        )
    );
}

#[test]
fn parentheses_and_tuples_keep_line_comments() {
    assert_eq!(
        lower("( {- a -} x -- e\n)"),
        Expr::Parens(Box::new(c((vec![block("a")], vec![line(" e")]), var("x"))))
    );
    assert_eq!(
        lower("( x -- e\n , y {- b -} )"),
        Expr::Tuple(
            vec![
                c((vec![], vec![line(" e")]), var("x")),
                c((vec![], vec![block("b")]), var("y")),
            ],
            true
        )
    );
    assert_eq!(lower("( {- u -} )"), Expr::Unit(vec![block("u")]));
}

#[test]
fn lists_and_records() {
    assert_eq!(
        lower("[ 1 -- e\n , {- x -} 2 {- t -} ]"),
        Expr::List {
            terms: Sequence(vec![
                c((vec![], vec![], Some(" e".to_owned())), int(1)),
                c((vec![], vec![block("x")], None), int(2)),
            ]),
            trailing: vec![block("t")],
            force_multiline: ForceMultiline(true),
        }
    );
    assert_eq!(
        lower("[ {- e -} ]"),
        Expr::List {
            terms: Sequence(Vec::new()),
            trailing: vec![block("e")],
            force_multiline: ForceMultiline(false),
        }
    );
    assert_eq!(
        lower("[ {- a -} 1 .. 2 {- b -} ]"),
        Expr::Range(
            Box::new(c((vec![block("a")], vec![]), int(1))),
            Box::new(c((vec![], vec![block("b")]), int(2))),
            false
        )
    );
    assert_eq!(
        lower("{ {- b -} r | a {- k -} = {- v -} 1 }"),
        Expr::Record {
            base: Some(c((vec![block("b")], vec![]), Lower("r".to_owned()))),
            fields: Sequence(vec![c(
                (vec![], vec![], None),
                Pair {
                    key: c(vec![block("k")], Lower("a".to_owned())),
                    value: c(vec![block("v")], int(1)),
                    force_multiline: ForceMultiline(false),
                }
            )]),
            trailing: vec![],
            force_multiline: ForceMultiline(false),
        }
    );
}

#[test]
fn if_chains() {
    assert_eq!(
        lower("if a then 1 {- p -} else {- q -} if b then 2 else {- r -} 3"),
        Expr::If(
            Box::new(IfClause {
                condition: c((vec![], vec![]), var("a")),
                body: c((vec![], vec![block("p")]), int(1)),
            }),
            vec![c(
                vec![block("q")],
                IfClause {
                    condition: c((vec![], vec![]), var("b")),
                    body: c((vec![], vec![]), int(2)),
                }
            )],
            Box::new(c(vec![block("r")], int(3)))
        )
    );
}

#[test]
fn case_and_lambda() {
    assert_eq!(
        lower("case {- s -} x of\n  {- p -} _ {- a -} -> {- b -} 1"),
        Expr::Case(
            Box::new(c((vec![block("s")], vec![]), var("x"))),
            false,
            vec![CaseBranch {
                before_pattern: vec![block("p")],
                before_arrow: vec![block("a")],
                after_arrow: vec![block("b")],
                pattern: Pattern::Anything,
                body: int(1),
            }]
        )
    );
    assert_eq!(
        lower("\\ {- a -} x {- b -} -> {- c -} x"),
        Expr::Lambda(
            vec![c(vec![block("a")], Pattern::Var(Lower("x".to_owned())))],
            vec![block("b"), block("c")],
            Box::new(var("x")),
            false
        )
    );
}

#[test]
fn let_keeps_comments_as_declarations() {
    let text =
        "let\n -- a\n f : Int\n -- b\n f {- c -} x = {- d -} x\n -- e\n (y, _) = z\n in {- f -} f";
    let Expr::Let(declarations, before_body, body) = lower(text) else {
        panic!()
    };
    assert_eq!(before_body, [block("f")]);
    assert_eq!(*body, var("f"));
    let common = |d| LetDeclaration::Common(Box::new(d));
    let int_type = Type::Construction(
        TypeConstructor::Named((vec![], Upper("Int".to_owned()))),
        vec![],
        ForceMultiline(false),
    );
    let y = Pattern::Var(Lower("y".to_owned()));
    assert_eq!(
        declarations,
        [
            LetDeclaration::Comment(line(" a")),
            common(CommonDeclaration::TypeAnnotation(
                c(vec![], Ref::Var((), Lower("f".to_owned()))),
                c(vec![], int_type)
            )),
            LetDeclaration::Comment(line(" b")),
            common(CommonDeclaration::Definition(
                Pattern::Var(Lower("f".to_owned())),
                vec![c(vec![block("c")], Pattern::Var(Lower("x".to_owned())))],
                vec![block("d")],
                var("x")
            )),
            LetDeclaration::Comment(line(" e")),
            common(CommonDeclaration::Definition(
                Pattern::Tuple(vec![
                    c((vec![], vec![]), y),
                    c((vec![], vec![]), Pattern::Anything)
                ]),
                vec![],
                vec![],
                var("z")
            )),
        ]
    );
}
