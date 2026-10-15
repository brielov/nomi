use super::*;
use crate::test_support::{block, line, lowered, names, upper};
use crate::tree::{
    Around, C1, CommentedMap, DetailedListing, Docs, ForceMultiline, Header, ImportKey,
    ImportMethod, Listing, SourceSetting, SourceTag,
};

/// Every comment in `text`, in order.
fn comments(text: &str) -> Comments {
    let root = nomi_syntax::parse_module(text).syntax();
    let mut cx = Cursor::new(&root, text);
    cx.take(TextSize::of(text))
}

fn values(items: &[(&str, Comments, Comments)]) -> CommentedMap<Lower, ()> {
    items
        .iter()
        .map(|(n, before, after)| {
            let comments = Around::new(before.clone(), after.clone());
            (Lower((*n).to_owned()), c(comments, ()))
        })
        .collect()
}

fn exports(module: &Module) -> &Listing<DetailedListing> {
    &module
        .header
        .as_ref()
        .unwrap()
        .exports
        .as_ref()
        .unwrap()
        .value
}

fn import<'a>(module: &'a Module, name: &[&str]) -> &'a C1<ImportMethod> {
    let key = ImportKey {
        module: names(name),
        group: 0,
    };
    &module.imports.value[&key]
}

#[test]
fn comments_read_as_upstream_reads_them() {
    let text = "--}\n--}  \n--} x\n-- y\n{--}\n{-- t -}\n{- -}\n{-a -}\n";
    assert_eq!(
        comments(&format!("module A exposing (a)\n{text}a = 1\n")),
        [
            Comment::TrickCloser,
            Comment::TrickCloser,
            line("} x"),
            line(" y"),
            Comment::TrickOpener,
            Comment::TrickBlock("t".to_owned()),
            Comment::Block(Vec::new()),
            block("a"),
        ]
    );
}

#[test]
fn block_comments_lose_their_shared_indentation() {
    let text = "module A exposing (a)\n{- one\n      two\n\n    three {- in -}\n   -}\na = 1\n";
    let lines = ["one", "  two", "", "three {- in -}"];
    assert_eq!(
        comments(text),
        [Comment::Block(lines.map(str::to_owned).to_vec())]
    );
}

#[test]
fn header_comments() {
    let module =
        lowered("-- a\nport {- b -} module {- c -} A.B {- d -} exposing {- e -} (..)\n\na = 1\n");
    assert_eq!(module.initial_comments, [line(" a")]);
    assert_eq!(
        module.header,
        Some(Header {
            source_tag: SourceTag::Port(vec![block("b")]),
            name: c(
                Around::new(vec![block("c")], Vec::new()),
                names(&["A", "B"])
            ),
            settings: None,
            exports: Some(c(
                Around::new(vec![block("d")], vec![block("e")]),
                Listing::Open(Around::default())
            )),
        })
    );
}

#[test]
fn header_of_elm_0_16() {
    let module = lowered("module A {- x -} (a) {- y -} where\n\na = 1\n");
    let header = module.header.unwrap();
    assert_eq!(
        header.name,
        c(Around::new(Vec::new(), vec![block("x")]), names(&["A"]))
    );
    let exports = header.exports.unwrap();
    assert_eq!(exports.comments, Around::new(vec![block("y")], Vec::new()));
    let Listing::Explicit(listing, ForceMultiline(false)) = exports.value else {
        panic!("{:?}", exports.value)
    };
    assert_eq!(listing.values, values(&[("a", vec![], vec![])]));
}

#[test]
fn effect_settings() {
    let text = "effect module A where { command = {- k -} MyCmd } exposing (a)\n\na = 1\n";
    let header = lowered(text).header.unwrap();
    assert_eq!(header.source_tag, SourceTag::Effect(Vec::new()));
    assert_eq!(
        header.settings,
        Some(C2::bare(vec![SourceSetting {
            key: C2::bare(Lower("command".to_owned())),
            value: c(Around::new(vec![block("k")], Vec::new()), upper("MyCmd")),
        }]))
    );
}

#[test]
fn listing_items_keep_comments_ending_their_lines() {
    let module = lowered(
        "module A exposing\n    ( a -- one\n    , b {- two -}\n    , C(..)\n    , D {- i -} (E, F -- f\n      )\n    )\n\na = 1\n",
    );
    let Listing::Explicit(listing, ForceMultiline(true)) = exports(&module) else {
        panic!("{:?}", exports(&module))
    };
    assert_eq!(
        listing.values,
        values(&[
            ("a", vec![], vec![line(" one")]),
            ("b", vec![], vec![block("two")]),
        ])
    );
    assert_eq!(
        listing.types[&upper("C")],
        C2::bare(c(Vec::new(), Listing::Open(Around::default())))
    );
    let ctors = [
        (upper("E"), C2::bare(())),
        (upper("F"), c(Around::new(Vec::new(), vec![line(" f")]), ())),
    ];
    assert_eq!(
        listing.types[&upper("D")],
        C2::bare(c(
            vec![block("i")],
            Listing::Explicit(ctors.into_iter().collect(), ForceMultiline(true))
        ))
    );
}

#[test]
fn an_item_over_lines_ends_no_line_comment() {
    let module = lowered("module A exposing (A\n  (..) -- c\n  )\n\na = 1\n");
    let Listing::Explicit(listing, ForceMultiline(true)) = exports(&module) else {
        panic!("{:?}", exports(&module))
    };
    assert_eq!(
        listing.types[&upper("A")].comments,
        Around::new(vec![], vec![line(" c")])
    );
    // Only `withEol` reads one, so the comment reaches the list as a
    // comment, not as the item's own.
    assert!(matches!(
        listing.types[&upper("A")].value.value,
        Listing::Open(..)
    ));
}

#[test]
fn comments_between_imports_go_before_them_all() {
    let module = lowered(
        "module A exposing (a)\n\n{-| Docs. -}\n-- h\nimport B\n-- between\nimport {- n -} C as {- p -} D exposing (x)\n-- after\na = 1\n",
    );
    assert_eq!(
        module.docs,
        Some(Docs {
            blocks: crate::markdown::parse("Docs.", &mut |_, _| None),
            source: "{-| Docs. -}".to_owned(),
        })
    );
    assert_eq!(module.imports.comments, [line(" h"), line(" between")]);
    assert_eq!(
        module.body[0],
        TopLevelStructure::BodyComment(line(" after"))
    );
    assert_eq!(
        import(&module, &["B"]).value.exposed,
        C2::bare(Listing::Closed)
    );
    let c_import = import(&module, &["C"]);
    assert_eq!(c_import.comments, [block("n")]);
    assert_eq!(
        c_import.value.alias,
        Some(c(Around::new(Vec::new(), vec![block("p")]), upper("D")))
    );
}

#[test]
fn without_imports_comments_before_the_body_go_before_imports() {
    let module = lowered("module A exposing (a)\n\n-- c\n\na = 1\n");
    assert_eq!(module.imports.comments, [line(" c")]);
    assert!(matches!(module.body[..], [TopLevelStructure::Entry(_)]));
}

#[test]
fn imports_of_one_module_merge_later_first() {
    let module = lowered(
        "module A exposing (a)\n\nimport {- 1 -} B exposing (a {- x -})\nimport {- 2 -} B as C exposing (b, a {- y -}, T(U), T {- i -} (V))\n\na = 1\n",
    );
    let b = import(&module, &["B"]);
    assert_eq!(b.comments, [block("2"), block("1")]);
    assert_eq!(b.value.alias, Some(C2::bare(upper("C"))));
    let Listing::Explicit(listing, ForceMultiline(false)) = &b.value.exposed.value else {
        panic!("{:?}", b.value.exposed)
    };
    assert_eq!(
        listing.values,
        values(&[
            ("a", vec![], vec![block("y"), block("x")]),
            ("b", vec![], vec![])
        ])
    );
    let Listing::Explicit(ctors, ForceMultiline(false)) = &listing.types[&upper("T")].value.value
    else {
        panic!("{:?}", listing.types)
    };
    assert_eq!(listing.types[&upper("T")].value.comments, [block("i")]);
    assert_eq!(
        ctors.keys().cloned().collect::<Vec<_>>(),
        names(&["U", "V"])
    );
}

#[test]
fn a_value_listed_twice_keeps_its_last_comments() {
    let module = lowered("module A exposing (a {- 1 -}, (-), a {- 2 -})\n\na = 1\n");
    let Listing::Explicit(listing, ForceMultiline(false)) = exports(&module) else {
        panic!("{:?}", exports(&module))
    };
    assert_eq!(listing.values, values(&[("a", vec![], vec![block("2")])]));
    assert!(listing.operators.contains_key(&Symbol("-".to_owned())));
}

#[test]
fn imports_without_parentheses() {
    let module = lowered(
        "module A exposing (a)\n\nimport B exposing a, b -- c\nimport C exposing ..\n\na = 1\n",
    );
    let Listing::Explicit(listing, ForceMultiline(false)) =
        &import(&module, &["B"]).value.exposed.value
    else {
        panic!()
    };
    assert_eq!(
        listing.values,
        values(&[("a", vec![], vec![]), ("b", vec![], vec![line(" c")])])
    );
    assert_eq!(
        import(&module, &["C"]).value.exposed,
        C2::bare(Listing::Open(Around::default()))
    );
}
