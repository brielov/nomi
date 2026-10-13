use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxElement, parse_module};

use super::*;

/// The HIR of `text`, and its tree.
fn with_root(text: &str) -> (Module, SyntaxNode) {
    let root = parse_module(text).syntax();
    (lower(&ast::Module::cast(root.clone()).unwrap()), root)
}

/// The text of `ptr`'s node, found again in `root`'s tree, without the
/// trivia around it.
fn text(root: &SyntaxNode, ptr: NodePtr) -> String {
    let start = match root.covering_element(ptr.range) {
        SyntaxElement::Node(node) => node,
        SyntaxElement::Token(token) => token.parent().unwrap(),
    };
    let node = (start.ancestors())
        .find(|n| n.kind() == ptr.kind && n.text_range() == ptr.range)
        .unwrap();
    node.text().to_string().trim().to_owned()
}

const MODULE: &str = "port module Main exposing (main, Msg(..), (+++))

{-| The app. -}

import Html as H exposing (text)
import Dict


type Msg
    = Got Int
    | Lost


type alias Point =
    { x : Int, y : Float }


port send : String -> Cmd msg


main : H.Html msg
main =
    let
        ( a, b ) =
            ( 1, -2.5 )

        f x =
            x + 1 * 2
    in
    if a == 1 then
        text \"one\"

    else if b > 0 then
        text (String.fromInt (f a))

    else
        case Just a of
            Just (n :: _) as all ->
                .x { p | x = 'c' }

            _ ->
                (\\_ -> [ () ]) 0
";

#[test]
fn a_module_lowers_as_upstream_parses_it() {
    let (module, root) = with_root(MODULE);
    assert_eq!(verify(&module, &root), Vec::<String>::new());
    assert_eq!(module.name.as_ref().unwrap().text, "Main");
    assert_eq!(module.kind, ModuleKind::Port);
    let Exposing::Explicit(exposed) = &module.exports else {
        panic!("{:?}", module.exports);
    };
    assert_eq!(exposed.len(), 3);
    assert!(matches!(&exposed[1], Exposed::Upper(name, Some(_)) if name.text == "Msg"));
    assert!(matches!(&exposed[2], Exposed::Operator(op) if op.text == "+++"));
    assert_eq!(module.docs.as_ref().unwrap().text, "{-| The app. -}");
    let imports: Vec<_> = module
        .imports
        .iter()
        .map(|i| {
            (
                i.name.text.as_str(),
                i.alias.as_ref().map(|a| a.text.as_str()),
                i.exposing.is_some(),
            )
        })
        .collect();
    assert_eq!(imports, [("Html", Some("H"), true), ("Dict", None, false)]);
    assert_eq!(module.unions[0].variants.len(), 2);
    assert_eq!(module.aliases[0].name.text, "Point");
    assert_eq!(module.ports[0].name.text, "send");
    assert_eq!(module.values.len(), 1);
    let Def::Define {
        name,
        annotation,
        body,
        ..
    } = &module.defs[module.values[0]]
    else {
        panic!();
    };
    assert_eq!(name.text, "main");
    assert!(annotation.is_some());
    let Expr::Let { defs, body } = &module.exprs[*body] else {
        panic!();
    };
    assert!(matches!(module.defs[defs[0]], Def::Destruct { .. }));
    // `else if` flattens; the last `else` stays.
    let Expr::If {
        branches,
        otherwise,
    } = &module.exprs[*body]
    else {
        panic!();
    };
    assert_eq!(branches.len(), 2);
    assert!(matches!(module.exprs[*otherwise], Expr::Case { .. }));
    // Parentheses go: the call's argument is the inner call.
    let Expr::Call { args, .. } = &module.exprs[branches[1].1] else {
        panic!();
    };
    assert!(matches!(module.exprs[args[0]], Expr::Call { .. }));
    assert_eq!(
        text(&root, module.source_map.exprs[args[0].index()].unwrap()),
        "String.fromInt (f a)"
    );
}

#[test]
fn a_module_without_a_header_that_declares_ports_is_a_port_module() {
    let module = lowered("port send : String -> Cmd msg\n\nmain = 1\n");
    assert_eq!(module.kind, ModuleKind::Port);
    assert_eq!(module.name, None);
    let module = lowered("main = 1\n");
    assert_eq!(module.kind, ModuleKind::Plain);
}

#[test]
fn chains_stay_flat_and_names_keep_their_qualifiers() {
    let (module, root) =
        with_root("module M exposing (x)\n\n\nx =\n    a + b * Dict.empty |> List.map f\n");
    assert_eq!(verify(&module, &root), Vec::<String>::new());
    let (_, Expr::Binops { operands, ops }) = module
        .exprs
        .iter()
        .find(|(_, e)| matches!(e, Expr::Binops { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(
        ops.iter().map(|o| o.text.as_str()).collect::<Vec<_>>(),
        ["+", "*", "|>"]
    );
    assert_eq!(operands.len(), 4);
    assert_eq!(
        module.exprs[operands[2]],
        Expr::Var {
            qualifier: Some("Dict".to_owned()),
            name: "empty".to_owned(),
            kind: VarType::Low
        }
    );
}

#[test]
fn every_id_maps_to_its_construct() {
    let (module, root) = with_root(MODULE);
    for (id, expr) in module.exprs.iter() {
        let node = text(&root, module.source_map.exprs[id.index()].unwrap());
        match expr {
            Expr::Var { name, .. } => assert!(node.ends_with(name.as_str()), "{node}"),
            Expr::Int(n) | Expr::Float(n) | Expr::Str(n) | Expr::Chr(n) => assert_eq!(&node, n),
            Expr::Unit => assert_eq!(node, "()"),
            _ => {}
        }
    }
    for (id, pattern) in module.patterns.iter() {
        if let Pattern::Var(name) = pattern {
            let ptr = module.source_map.patterns[id.index()].unwrap();
            assert_eq!(&text(&root, ptr), name);
        }
    }
}

#[test]
fn what_is_missing_is_missing() {
    let (module, root) = with_root("module M exposing (x)\n\n\nx =\n    f (\n\ny = [1..2]\n");
    let problems = verify(&module, &root);
    assert!(
        problems.iter().any(|p| p.contains("missing")),
        "{problems:?}"
    );
    assert!(module.exprs.iter().any(|(_, e)| *e == Expr::Missing));
}

#[test]
fn deep_nesting_lowers_and_drops() {
    nomi_base::on_core_stack(|| {
        let depth = 20_000;
        let text = format!(
            "module M exposing (x)\n\n\nx =\n    {}1{}\n",
            "(".repeat(depth),
            ")".repeat(depth)
        );
        let deep_list = format!(
            "module M exposing (x)\n\n\nx =\n    {}1{}\n",
            "[".repeat(depth),
            "]".repeat(depth)
        );
        for text in [text, deep_list] {
            let (module, root) = with_root(&text);
            assert_eq!(verify(&module, &root), Vec::<String>::new());
            drop(module);
        }
    });
}
