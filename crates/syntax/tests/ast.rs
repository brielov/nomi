use nomi_syntax::SyntaxKind;
use nomi_syntax::ast::{self, AstNode};

fn module(text: &str) -> ast::Module {
    let parse = nomi_syntax::parse_module(text);
    ast::Module::cast(parse.syntax()).unwrap()
}

fn expr(text: &str) -> ast::Expr {
    let parse = nomi_syntax::parse_expr(text);
    let fragment = ast::ExprFragment::cast(parse.syntax()).unwrap();
    fragment.expr().unwrap()
}

fn text(node: &impl AstNode) -> String {
    node.syntax().text().to_string()
}

#[test]
fn every_node_kind_has_a_view() {
    for &kind in SyntaxKind::ALL {
        let expected = kind.is_node() && !matches!(kind, SyntaxKind::Missing | SyntaxKind::Error);
        assert_eq!(ast::has_view(kind), expected, "{kind:?}");
    }
}

#[test]
fn modules_hold_their_parts() {
    let m = module(
        "module Main exposing (main, Model(..))\n\nimport Html as H\n\n{-| The entry. -}\nmain =\n    1\n",
    );
    let header = m.module_header().unwrap();
    assert_eq!(text(&header.module_name().unwrap()), "Main");
    let exposed: Vec<_> = header
        .exposing_list()
        .unwrap()
        .items()
        .map(|e| text(&e))
        .collect();
    assert_eq!(exposed, ["main", "Model(..)"]);
    let import = m.imports().next().unwrap();
    assert_eq!(
        import
            .import_alias()
            .unwrap()
            .upper_ident_token()
            .unwrap()
            .text(),
        "H"
    );
    let Some(ast::Decl::ValueDecl(main)) = m.decls().next() else {
        panic!("a value declaration")
    };
    assert_eq!(
        main.doc_comment_token().unwrap().text(),
        "{-| The entry. -}"
    );
    let definition = main.definition().unwrap();
    assert_eq!(definition.lower_ident_token().unwrap().text(), "main");
    assert_eq!(text(&definition.expr().unwrap()), "1");
}

#[test]
fn if_branches_are_told_apart_by_their_keywords() {
    let ast::Expr::IfExpr(e) = expr("if a then b else c") else {
        panic!()
    };
    assert_eq!(text(&e.condition().unwrap()), "a");
    assert_eq!(text(&e.then_branch().unwrap()), "b");
    assert_eq!(text(&e.else_branch().unwrap()), "c");
}

#[test]
fn a_missing_child_shifts_nothing() {
    let ast::Expr::IfExpr(e) = expr("if then b else c") else {
        panic!()
    };
    assert_eq!(e.condition(), None);
    assert_eq!(text(&e.then_branch().unwrap()), "b");
    assert_eq!(text(&e.else_branch().unwrap()), "c");
}

#[test]
fn calls_split_function_from_arguments() {
    let ast::Expr::CallExpr(e) = expr("f x (g y)") else {
        panic!()
    };
    assert_eq!(text(&e.function().unwrap()), "f");
    let arguments: Vec<_> = e.arguments().map(|a| text(&a)).collect();
    assert_eq!(arguments, ["x", "(g y)"]);
}

#[test]
fn binary_shapes_split_at_their_token() {
    let parse = nomi_syntax::parse_type("a -> b -> c");
    let ty = ast::TypeFragment::cast(parse.syntax())
        .unwrap()
        .ty()
        .unwrap();
    let ast::Type::FunctionType(f) = ty else {
        panic!()
    };
    assert_eq!(text(&f.param().unwrap()), "a");
    assert_eq!(text(&f.result().unwrap()), "b -> c");

    let parse = nomi_syntax::parse_pattern("x :: xs");
    let pattern = ast::PatternFragment::cast(parse.syntax())
        .unwrap()
        .pattern()
        .unwrap();
    let ast::Pattern::ConsPattern(p) = pattern else {
        panic!()
    };
    assert_eq!(text(&p.head().unwrap()), "x");
    assert_eq!(text(&p.tail().unwrap()), "xs");
}

#[test]
fn enums_cast_only_their_variants() {
    let e = expr("x");
    assert!(matches!(e, ast::Expr::NameExpr(_)));
    assert!(ast::Pattern::cast(e.syntax().clone()).is_none());
    let name = ast::NameExpr::cast(e.syntax().clone()).unwrap();
    assert_eq!(ast::Expr::from(name), e);
}

#[test]
fn a_missing_operand_is_none() {
    let parse = nomi_syntax::parse_type("-> b");
    let fragment = ast::TypeFragment::cast(parse.syntax()).unwrap();
    let Some(ast::Type::FunctionType(f)) = fragment.ty() else {
        panic!("{:#?}", parse.syntax())
    };
    assert_eq!(f.param(), None);
    assert_eq!(text(&f.result().unwrap()), "b");
}
