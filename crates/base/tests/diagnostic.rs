use nomi_base::diagnostic::doc::{
    Color, Line, comma_sep, fill_sep, hint, link, reflow, reflow_link, sep, simple_note, stack,
    text, vcat,
};
use nomi_base::diagnostic::{OverlappingEdits, Snippet};
use nomi_base::{Code, Diagnostic, Doc, Edit, FileId, Fix, Severity, Span, TextRange, TextSize};

const UNFINISHED_LET: Code = Code::new("syntax/unfinished-let");

fn span(file: u32, start: u32, end: u32) -> Span {
    Span::new(
        FileId::from_raw(file),
        TextRange::new(TextSize::new(start), TextSize::new(end)),
    )
}

#[test]
fn codes_are_lower_case_words() {
    for code in [
        "syntax",
        "syntax/unfinished-let",
        "lint/unused-import",
        "e2e/x1",
    ] {
        assert_eq!(Code::new(code).as_str(), code);
    }
    assert_eq!(UNFINISHED_LET.to_string(), "syntax/unfinished-let");
}

#[test]
fn malformed_codes_are_rejected() {
    for code in ["", "Syntax", "a--b", "a/", "/a", "a-/b", "a b", "a_b", "-a"] {
        let result = std::panic::catch_unwind(|| Code::new(code));
        assert!(result.is_err(), "{code:?} was accepted");
    }
}

#[test]
fn a_diagnostic_carries_its_parts() {
    let note = Doc::text("the let starts here");
    let fix = Fix::new("Add `in`", vec![Edit::new(span(0, 9, 9), " in")]).unwrap();
    let diagnostic = Diagnostic::error(
        UNFINISHED_LET,
        "UNFINISHED LET",
        span(0, 4, 9),
        stack([
            reflow("I was partway through a let."),
            Doc::snippet(span(0, 0, 9).range, span(0, 4, 9).range),
        ]),
    )
    .with_label(span(0, 0, 3), note.clone())
    .with_fix(fix.clone());
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(diagnostic.title, "UNFINISHED LET");
    assert_eq!(diagnostic.secondary[0].span, span(0, 0, 3));
    assert_eq!(diagnostic.secondary[0].note, note);
    assert_eq!(diagnostic.fixes, [fix]);
    assert_eq!(
        diagnostic.message.plain(),
        "I was partway through a let.[snippet]"
    );
    assert!(Severity::Hint < Severity::Warning && Severity::Warning < Severity::Error);
}

#[test]
fn text_is_never_empty_and_append_drops_empties() {
    assert_eq!(Doc::text(""), Doc::Empty);
    assert_eq!(Doc::empty().append(text("a")), text("a"));
    assert_eq!(text("a").append(Doc::empty()), text("a"));
    assert_eq!(text("a").space(text("b")).plain(), "a b");
}

#[test]
#[should_panic(expected = "newline")]
fn text_rejects_newlines() {
    let _ = Doc::text("a\nb");
}

#[test]
fn combinators_build_upstreams_shapes() {
    assert_eq!(vcat(Vec::new()), Doc::Empty);
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
    assert_eq!(Doc::softline(), Doc::Line(Line::Space));
    assert_eq!(text("x").indent(2).plain(), "  x");
    assert_eq!(
        text("x").color(Color::Red).bold().underline(),
        Doc::Underline(Box::new(Doc::Bold(Box::new(Doc::Color(
            Color::Red,
            Box::new(text("x"))
        )))))
    );
}

#[test]
fn comma_sep_matches_elm() {
    let names = |n: usize| (0..n).map(|i| text(format!("x{i}"))).collect::<Vec<_>>();
    let plain = |docs: Vec<Doc>| docs.iter().map(Doc::plain).collect::<Vec<_>>().join(" ");
    assert_eq!(plain(comma_sep("or", names(1))), "x0");
    assert_eq!(plain(comma_sep("or", names(2))), "x0 or x1");
    assert_eq!(plain(comma_sep("or", names(3))), "x0, x1, or x2");
    // The comma sits outside the item's styling.
    let styled = comma_sep("and", vec![text("a").bold(), text("b"), text("c")]);
    assert_eq!(styled[0], text("a").bold().append(text(",")));
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
        hint([text("Try"), text("this.")]),
        fill_sep([
            text("Hint").underline().append(text(":")),
            text("Try"),
            text("this.")
        ])
    );
    assert_eq!(simple_note("It is fine.").plain(), "Note: It is fine.");
    assert_eq!(
        Doc::snippet(span(0, 0, 4).range, span(0, 1, 2).range),
        Doc::Snippet(Snippet {
            region: span(0, 0, 4).range,
            highlight: span(0, 1, 2).range
        })
    );
    assert_eq!(Color::Red.json_name(), "RED");
    assert_eq!(Color::DullRed.json_name(), "red");
}

#[test]
fn fix_edits_are_sorted_and_applied() {
    let fix = Fix::new(
        "Rename",
        vec![
            Edit::new(span(0, 6, 9), "qux"),
            Edit::new(span(1, 0, 0), "elsewhere"),
            Edit::new(span(0, 0, 3), "baz"),
            // Touching the previous edit is fine.
            Edit::new(span(0, 3, 3), "!"),
        ],
    )
    .unwrap();
    let starts: Vec<_> = fix
        .edits()
        .iter()
        .map(|e| (e.span.file.raw(), u32::from(e.span.range.start())))
        .collect();
    assert_eq!(starts, [(0, 0), (0, 3), (0, 6), (1, 0)]);
    assert_eq!(fix.apply(FileId::from_raw(0), "foo = bar"), "baz! = qux");
    assert_eq!(fix.apply(FileId::from_raw(1), "x"), "elsewherex");
    assert_eq!(fix.apply(FileId::from_raw(2), "untouched"), "untouched");
}

#[test]
fn overlapping_edits_are_rejected() {
    let overlap = |a: Span, b: Span| Fix::new("f", vec![Edit::new(a, "a"), Edit::new(b, "b")]);
    let err: OverlappingEdits = overlap(span(0, 0, 4), span(0, 3, 5)).unwrap_err();
    assert_eq!(
        (err.first.insert.as_str(), err.second.insert.as_str()),
        ("a", "b")
    );
    assert!(overlap(span(0, 0, 4), span(0, 1, 2)).is_err());
    // Two insertions at one point have no defined order.
    assert!(overlap(span(0, 2, 2), span(0, 2, 2)).is_err());
    assert!(overlap(span(0, 0, 4), span(1, 0, 4)).is_ok());
    assert!(overlap(span(0, 0, 2), span(0, 2, 4)).is_ok());
}
