use super::*;
use crate::diagnostic::doc::{fill_sep, reflow, stack, text, vcat};

fn plain(doc: &Doc, width: usize) -> String {
    render(doc, Compat::V0_19_1, width, Styles::Plain)
}

#[test]
fn fill_sep_wraps_where_the_next_word_would_not_fit() {
    let doc = reflow("aaa bbb ccc ddd");
    assert_eq!(plain(&doc, 11), "aaa bbb ccc\nddd");
    assert_eq!(plain(&doc, 10), "aaa bbb\nccc ddd");
}

#[test]
fn a_group_fits_with_what_follows_it_up_to_the_next_newline() {
    // The group would fit alone, but not with the text after it.
    let group = text("a").append(Doc::softline()).append(text("b")).group();
    let doc = group
        .append(text("cccc"))
        .append(Doc::line())
        .append(text("dddddd"));
    assert_eq!(plain(&doc, 6), "a\nbcccc\ndddddd");
    assert_eq!(plain(&doc, 7), "a bcccc\ndddddd");
}

#[test]
fn a_hard_line_keeps_its_group_broken() {
    let doc = text("a").append(Doc::line()).append(text("b")).group();
    assert_eq!(plain(&doc, 80), "a\nb");
}

#[test]
fn vcat_runs_together_in_a_group_on_one_line() {
    // `vcat` joins with `linebreak`, as wl-pprint's does.
    let doc = vcat([text("a"), text("b")]).group();
    assert_eq!(plain(&doc, 80), "ab");
    assert_eq!(plain(&doc, 1), "a\nb");
}

#[test]
fn newlines_indent_even_blank_lines() {
    let doc = stack([text("a"), text("b")]).indent(2);
    assert_eq!(plain(&doc, 80), "  a\n  \n  b");
}

#[test]
fn align_nests_at_the_current_column() {
    let doc = text("ab ").append(vcat([text("c"), text("d")]).align());
    assert_eq!(plain(&doc, 80), "ab c\n   d");
}

#[test]
fn fill_pads_to_its_width() {
    let doc = vcat([
        text("elm/core").fill(10).append(text("1.0.5")),
        text("elm/browser").fill(10).append(text("1.0.2")),
    ]);
    assert_eq!(plain(&doc, 80), "elm/core  1.0.5\nelm/browser1.0.2");
}

#[test]
fn fill_counts_toward_fitting() {
    let doc = fill_sep([text("a").fill(4), text("b")]);
    assert_eq!(plain(&doc, 6), "a    b");
    assert_eq!(plain(&doc, 5), "a   \nb");
}

#[test]
fn links_name_the_compatibility_target() {
    let doc = Doc::link("init");
    assert_eq!(
        render(&doc, Compat::V0_19_1, WIDTH, Styles::Plain),
        "<https://elm-lang.org/0.19.1/init>"
    );
    assert_eq!(
        render(&doc, Compat::V0_19_3, WIDTH, Styles::Plain),
        "<https://elm-lang.org/0.19.3/init>"
    );
}

#[test]
fn styles_are_escapes_restored_as_wl_pprint_restores_them() {
    let doc = text("a")
        .append(text("b").color(Color::Red))
        .append(text("c"))
        .color(Color::DullCyan)
        .underline();
    assert_eq!(plain(&doc, 80), "abc");
    assert_eq!(
        render(&doc, Compat::V0_19_1, 80, Styles::Ansi),
        "\x1b[4m\x1b[36ma\x1b[91mb\x1b[0;36;4mc\x1b[0;4m\x1b[0m"
    );
}
