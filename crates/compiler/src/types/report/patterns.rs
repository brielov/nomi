//! Patterns that match the wrong type (`toPatternReport`).

use super::*;

/// `toPatternReport`.
pub(super) fn pattern_report(
    cx: &ReportContext<'_>,
    pattern_region: TextRange,
    category: &PCategory,
    actual: &t::Type,
    expected: &PExpected<t::Type>,
) -> (&'static str, TextRange, Doc) {
    let localizer = cx.localizer;
    let doc = match expected {
        PExpected::PNoExpectation(expected) => snip(
            cx,
            pattern_region,
            None,
            reflow("This pattern is being used in an unexpected way:"),
            pattern_comparison(
                localizer,
                actual,
                expected,
                &add_pattern_category("It is", category),
                "But it needs to match:",
                Vec::new(),
            ),
        ),
        PExpected::PFromContext(region, context, expected) => {
            let (problem, details) = match context {
                PContext::PTypedArg(name, index) => {
                    let ith = ordinal(index + 1);
                    (
                        reflow(&format!("The {ith} argument to `{name}` is weird.")),
                        pattern_comparison(
                            localizer,
                            actual,
                            expected,
                            &add_pattern_category(
                                "The argument is a pattern that matches",
                                category,
                            ),
                            &format!(
                                "But the type annotation on `{name}` says the {ith} argument should be:"
                            ),
                            Vec::new(),
                        ),
                    )
                }
                PContext::PCaseMatch(0) => (
                    reflow("The 1st pattern in this `case` causing a mismatch:"),
                    pattern_comparison(
                        localizer,
                        actual,
                        expected,
                        &add_pattern_category("The first pattern is trying to match", category),
                        "But the expression between `case` and `of` is:",
                        vec![reflow(
                            "These can never match! Is the pattern the problem? Or is it the expression?",
                        )],
                    ),
                ),
                PContext::PCaseMatch(index) => {
                    let ith = ordinal(index + 1);
                    (
                        reflow(&format!(
                            "The {ith} pattern in this `case` does not match the previous ones."
                        )),
                        pattern_comparison(
                            localizer,
                            actual,
                            expected,
                            &add_pattern_category(
                                &format!("The {ith} pattern is trying to match"),
                                category,
                            ),
                            "But all the previous patterns match:",
                            vec![link(
                                "Note",
                                "A `case` expression can only handle one type of value, so you may want to use",
                                "custom-types",
                                "to handle “mixing” types.",
                            )],
                        ),
                    )
                }
                PContext::PCtorArg(name, index) => {
                    let ith = ordinal(index + 1);
                    (
                        reflow(&format!("The {ith} argument to `{name}` is weird.")),
                        pattern_comparison(
                            localizer,
                            actual,
                            expected,
                            &add_pattern_category("It is trying to match", category),
                            &format!("But `{name}` needs its {ith} argument to be:"),
                            Vec::new(),
                        ),
                    )
                }
                PContext::PListEntry(index) => {
                    let ith = ordinal(index + 1);
                    (
                        reflow(&format!(
                            "The {ith} pattern in this list does not match all the previous ones:"
                        )),
                        pattern_comparison(
                            localizer,
                            actual,
                            expected,
                            &add_pattern_category(
                                &format!("The {ith} pattern is trying to match"),
                                category,
                            ),
                            "But all the previous patterns in the list are:",
                            vec![link(
                                "Hint",
                                "Everything in a list must be the same type of value. This way, we never run into unexpected values partway through a List.map, List.foldl, etc. Read",
                                "custom-types",
                                "to learn how to “mix” types.",
                            )],
                        ),
                    )
                }
                PContext::PTail => (
                    reflow("The pattern after (::) is causing issues."),
                    pattern_comparison(
                        localizer,
                        actual,
                        expected,
                        &add_pattern_category(
                            "The pattern after (::) is trying to match",
                            category,
                        ),
                        "But it needs to match lists like this:",
                        Vec::new(),
                    ),
                ),
            };
            snip(cx, *region, Some(pattern_region), problem, details)
        }
    };
    ("TYPE MISMATCH", pattern_region, doc)
}

/// `patternTypeComparison`: as `typeComparison`, the problem's hint
/// before the context's.
fn pattern_comparison(
    localizer: &Localizer,
    actual: &t::Type,
    expected: &t::Type,
    i_am_seeing: &str,
    instead_of: &str,
    context_hints: Vec<Doc>,
) -> Doc {
    let (actual_doc, expected_doc, problems) = to_comparison(localizer, actual, expected);
    let mut docs = vec![
        reflow(i_am_seeing),
        actual_doc.indent(4),
        reflow(instead_of),
        expected_doc.indent(4),
    ];
    docs.extend(problems_to_hint(&problems));
    docs.extend(context_hints);
    stack(docs)
}

/// `addPatternCategory`.
fn add_pattern_category(i_am_trying_to_match: &str, category: &PCategory) -> String {
    let what = match category {
        PCategory::PRecord => " record values of type:".to_owned(),
        PCategory::PUnit => " unit values:".to_owned(),
        PCategory::PTuple => " tuples of type:".to_owned(),
        PCategory::PList => " lists of type:".to_owned(),
        PCategory::PCtor(name) => format!(" `{name}` values of type:"),
        PCategory::PInt => " integers:".to_owned(),
        PCategory::PStr => " strings:".to_owned(),
        PCategory::PChr => " characters:".to_owned(),
        PCategory::PBool => " booleans:".to_owned(),
    };
    format!("{i_am_trying_to_match}{what}")
}
