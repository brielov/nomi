use nomi_syntax::{ast, parse_expr};

use super::super::{c, fragment};
use super::expr;
use crate::test_support::{block, line, lower as name, named, separated, upper};
use crate::tree::{
    AnnotatedName, ApplicationMultiline, Around, BinopsClause, C2, CaseBranch, CommonDeclaration,
    Expr, ForceMultiline, IfClause, IntRepr, LetDeclaration, LiteralValue, Multiline, Pair,
    Pattern, Ref, Sequence, Symbol,
};

/// The expression `text` holds, which must place every comment.
fn lower(text: &str) -> Expr {
    fragment(text, parse_expr, |f: ast::ExprFragment| f.expr(), expr)
}

fn var(name: &str) -> Expr {
    Expr::Var(Ref::Var(Vec::new(), self::name(name)))
}

fn int(value: i64) -> Expr {
    Expr::Literal(LiteralValue::Int(value, IntRepr::Decimal))
}

fn app(function: Expr, args: Vec<Expr>, multiline: ApplicationMultiline) -> Expr {
    Expr::App {
        function: Box::new(function),
        args: args.into_iter().map(|a| c(Vec::new(), a)).collect(),
        multiline,
    }
}

const JOINED: ApplicationMultiline = ApplicationMultiline::JoinFirst(Multiline::JoinAll);

#[test]
fn names_and_literals() {
    assert_eq!(
        lower("List.map"),
        Expr::Var(Ref::Var(vec![upper("List")], name("map")))
    );
    assert_eq!(lower("True"), Expr::Literal(LiteralValue::Bool(true)));
    assert_eq!(
        lower("Basics.True"),
        Expr::Var(Ref::Tag(vec![upper("Basics")], upper("True")))
    );
    assert_eq!(lower("-5"), int(-5));
    assert_eq!(lower("-x"), Expr::Negate(Box::new(var("x"))));
    assert_eq!(lower("(+)"), Expr::Var(Ref::Op(Symbol("+".to_owned()))));
    assert_eq!(lower("(,,)"), Expr::TupleFunction(3));
    assert_eq!(lower(".name"), Expr::AccessFunction(name("name")));
    assert_eq!(
        lower("a.b.c"),
        Expr::Access(
            Box::new(Expr::Access(Box::new(var("a")), name("b"))),
            name("c")
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
        Expr::App {
            function: Box::new(var("f")),
            args: vec![c(vec![block("x")], var("a"))],
            multiline: JOINED,
        }
    );
}

#[test]
fn operators_stay_flat() {
    assert_eq!(
        lower("a {- x -} + {- y -} b * c"),
        Expr::Binops {
            first: Box::new(var("a")),
            clauses: vec![
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
            multiline: ForceMultiline(false),
        }
    );
}

#[test]
fn parentheses_and_tuples_keep_line_comments() {
    assert_eq!(
        lower("( {- a -} x -- e\n)"),
        Expr::Parens(Box::new(c(
            Around::new(vec![block("a")], vec![line(" e")]),
            var("x")
        )))
    );
    assert_eq!(
        lower("( x -- e\n , y {- b -} )"),
        Expr::Tuple(
            vec![
                c(Around::new(vec![], vec![line(" e")]), var("x")),
                c(Around::new(vec![], vec![block("b")]), var("y")),
            ],
            ForceMultiline(true)
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
                c(separated(vec![], vec![], Some(" e")), int(1)),
                c(separated(vec![], vec![block("x")], None), int(2)),
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
        Expr::Range {
            low: Box::new(c(Around::new(vec![block("a")], vec![]), int(1))),
            high: Box::new(c(Around::new(vec![], vec![block("b")]), int(2))),
        }
    );
    assert_eq!(
        lower("{ {- b -} r | a {- k -} = {- v -} 1 }"),
        Expr::Record {
            base: Some(c(Around::new(vec![block("b")], vec![]), name("r"))),
            fields: Sequence(vec![c(
                separated(vec![], vec![], None),
                Pair {
                    key: c(vec![block("k")], name("a")),
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
        Expr::If {
            first: Box::new(IfClause {
                condition: C2::bare(var("a")),
                body: c(Around::new(vec![], vec![block("p")]), int(1)),
            }),
            else_ifs: vec![c(
                vec![block("q")],
                IfClause {
                    condition: C2::bare(var("b")),
                    body: C2::bare(int(2)),
                }
            )],
            else_body: Box::new(c(vec![block("r")], int(3))),
        }
    );
}

#[test]
fn case_and_lambda() {
    assert_eq!(
        lower("case {- s -} x of\n  {- p -} _ {- a -} -> {- b -} 1"),
        Expr::Case {
            subject: Box::new(c(Around::new(vec![block("s")], vec![]), var("x"))),
            multiline: ForceMultiline(false),
            branches: vec![CaseBranch {
                before_pattern: vec![block("p")],
                before_arrow: vec![block("a")],
                after_arrow: vec![block("b")],
                pattern: Pattern::Anything,
                body: int(1),
            }],
        }
    );
    assert_eq!(
        lower("\\ {- a -} x {- b -} -> {- c -} x"),
        Expr::Lambda {
            args: vec![c(vec![block("a")], Pattern::Var(name("x")))],
            before_body: vec![block("b"), block("c")],
            body: Box::new(var("x")),
            multiline: ForceMultiline(false),
        }
    );
}

#[test]
fn let_keeps_comments_as_declarations() {
    let text =
        "let\n -- a\n f : Int\n -- b\n f {- c -} x = {- d -} x\n -- e\n (y, _) = z\n in {- f -} f";
    let Expr::Let {
        declarations,
        before_body,
        body,
    } = lower(text)
    else {
        panic!()
    };
    assert_eq!(before_body, [block("f")]);
    assert_eq!(*body, var("f"));
    let common = |d| LetDeclaration::Common(Box::new(d));
    assert_eq!(
        declarations,
        [
            LetDeclaration::Comment(line(" a")),
            common(CommonDeclaration::TypeAnnotation {
                name: c(vec![], AnnotatedName::Var(name("f"))),
                ty: c(vec![], named(&[], "Int", vec![])),
            }),
            LetDeclaration::Comment(line(" b")),
            common(CommonDeclaration::Definition {
                name: Pattern::Var(name("f")),
                args: vec![c(vec![block("c")], Pattern::Var(name("x")))],
                before_body: vec![block("d")],
                body: var("x"),
            }),
            LetDeclaration::Comment(line(" e")),
            common(CommonDeclaration::Definition {
                name: Pattern::Tuple(vec![
                    C2::bare(Pattern::Var(name("y"))),
                    C2::bare(Pattern::Anything)
                ]),
                args: vec![],
                before_body: vec![],
                body: var("z"),
            }),
        ]
    );
}
