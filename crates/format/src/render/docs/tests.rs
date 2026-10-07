use super::*;

fn groups(groups: &[&[&str]]) -> Vec<Vec<String>> {
    groups
        .iter()
        .map(|g| g.iter().map(|n| (*n).to_owned()).collect())
        .collect()
}

#[test]
fn each_line_after_docs_is_a_group() {
    assert_eq!(
        documented("Intro.\n\n@docs a, b ,, c\nd, e\n@docs f\n\nG\n"),
        groups(&[&["a", "b", "c"], &["d", "e"], &["f"]])
    );
    assert_eq!(documented("  @docs a\n"), groups(&[&["a"]]));
    // A paragraph's line takes no `@docs` from the next one…
    assert_eq!(documented("Text\n@docs a\nb\n"), groups(&[&["a"], &["b"]]));
    // …and a docs line with no names is no group.
    assert_eq!(documented("@docs\n@docs ,\n"), groups(&[]));
}

#[test]
fn only_top_level_text_lists_names() {
    let text = "    @docs code\n\n```\n@docs fenced\n```\n> @docs quoted\n\n- @docs listed\n  @docs item\n\n# @docs header\n";
    assert_eq!(documented(text), groups(&[]));
    // Lazy continuation keeps indented text in a paragraph.
    assert_eq!(documented("@docs a\n    b\n"), groups(&[&["a"], &["b"]]));
    assert_eq!(
        documented("~~~~\n```\n@docs a\n~~~~\n@docs b\n"),
        groups(&[&["b"]])
    );
}
