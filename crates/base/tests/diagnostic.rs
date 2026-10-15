use nomi_base::Doc;
use nomi_base::diagnostic::doc::{
    Color, Conjunction, comma_sep, cyan, dull_yellow, fill_sep, from_chars, hint, labelled, link,
    red, reflow, reflow_link, sep, simple_note, stack, text, vcat, words,
};

#[test]
fn text_is_never_empty_and_append_drops_empties() {
    assert_eq!(text(""), Doc::empty());
    assert_eq!(Doc::empty().append(text("a")), text("a"));
    assert_eq!(text("a").append(Doc::empty()), text("a"));
    assert_eq!(text("a").space(text("b")).plain(), "a b");
}

#[test]
#[should_panic(expected = "newline")]
fn text_rejects_newlines() {
    let _ = text("a\nb");
}

#[test]
fn combinators_build_upstreams_shapes() {
    assert_eq!(vcat(Vec::new()), Doc::empty());
    assert_eq!(
        vcat([text("a"), text("b")]),
        text("a").append(Doc::linebreak()).append(text("b"))
    );
    assert_eq!(
        sep([text("a"), text("b")]),
        text("a").append(Doc::softline()).append(text("b")).group()
    );
    assert_eq!(
        fill_sep([text("a"), text("b")]),
        text("a").append(Doc::softline().group()).append(text("b"))
    );
    assert_eq!(
        reflow("  two\n words "),
        fill_sep([text("two"), text("words")])
    );
    assert_eq!(stack([text("a"), text("b")]).plain(), "ab");
    assert_eq!(text("x").indent(2).plain(), "  x");
    assert_eq!(red(""), Doc::empty().color(Color::Red));
    assert_eq!(cyan("x"), text("x").color(Color::Cyan));
    assert_eq!(dull_yellow(text("x")), text("x").color(Color::DullYellow));
    assert_eq!(
        words(" two\twords ").collect::<Vec<_>>(),
        [text("two"), text("words")]
    );
    assert_eq!(from_chars("a\nb"), vcat([text("a"), text("b")]));
}

#[test]
fn comma_sep_matches_elm() {
    let names = |n: usize| (0..n).map(|i| text(format!("x{i}"))).collect::<Vec<_>>();
    let plain = |docs: Vec<Doc>| docs.iter().map(Doc::plain).collect::<Vec<_>>().join(" ");
    assert_eq!(plain(comma_sep(Conjunction::Or, names(1))), "x0");
    assert_eq!(plain(comma_sep(Conjunction::Or, names(2))), "x0 or x1");
    assert_eq!(plain(comma_sep(Conjunction::Or, names(3))), "x0, x1, or x2");
    assert_eq!(plain(comma_sep(Conjunction::And, names(2))), "x0 and x1");
    // The comma sits outside the item's styling.
    let styled = comma_sep(
        Conjunction::And,
        vec![text("a").underline(), text("b"), text("c")],
    );
    assert_eq!(styled[0], text("a").underline().append(text(",")));
}

#[test]
fn links_and_hints_stay_symbolic() {
    assert_eq!(
        reflow_link("Read", "imports", "to learn more.").plain(),
        "Read <imports> to learn more."
    );
    let doc = link("Hint", "Read", "imports", "for more.");
    assert_eq!(doc.plain(), "Hint: Read <imports> for more.");
    assert_eq!(
        doc,
        labelled(
            "Hint",
            [
                text("Read"),
                Doc::link("imports"),
                text("for"),
                text("more.")
            ]
        )
    );
    assert_eq!(
        hint([text("Try"), text("this.")]),
        fill_sep([
            text("Hint").underline().append(text(":")),
            text("Try"),
            text("this.")
        ])
    );
    assert_eq!(simple_note("It is fine.").plain(), "Note: It is fine.");
    assert_eq!(Color::Red.json_name(), "RED");
    assert_eq!(Color::DullRed.json_name(), "red");
}
