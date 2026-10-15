//! Patterns that match the wrong type (`toPatternReport`).

use nomi_base::TextRange;
use nomi_base::diagnostic::doc::{Doc, link, ordinal, reflow};

use super::{HintOrder, mix_types, t, type_comparison};
use crate::report::{Report, ReportContext};
use crate::types::expected::{PCategory, PContext, PExpected};
use crate::types::localizer::Localizer;

/// `toPatternReport`.
#[expect(
    clippy::too_many_lines,
    reason = "upstream's `toPatternReport`, one message per context"
)]
pub(super) fn pattern_report(
    cx: &ReportContext<'_>,
    localizer: &Localizer,
    pattern_region: TextRange,
    category: &PCategory,
    actual: &t::Type,
    expected: &PExpected<t::Type>,
) -> Report {
    // `patternTypeComparison`: as `typeComparison`, the problem's hint
    // before the context's.
    let compare = |expected, matches: &str, instead_of: &str, hints: Vec<Doc>| {
        let i_am_seeing = add_pattern_category(matches, category);
        type_comparison(
            localizer,
            actual,
            expected,
            &i_am_seeing,
            instead_of,
            hints,
            HintOrder::ProblemFirst,
        )
    };
    let doc = match expected {
        PExpected::PNoExpectation(expected) => {
            let comparison = compare(expected, "It is", "But it needs to match:", Vec::new());
            let problem = reflow("This pattern is being used in an unexpected way:");
            cx.snippet(pattern_region, None, problem, comparison)
        }
        PExpected::PFromContext {
            region,
            context,
            ty: expected,
        } => {
            let (problem, details) = match context {
                PContext::PTypedArg(name, index) => {
                    let ith = ordinal(index + 1);
                    (
                        reflow(&format!("The {ith} argument to `{name}` is weird.")),
                        compare(
                            expected,
                            "The argument is a pattern that matches",
                            &format!(
                                "But the type annotation on `{name}` says the {ith} argument should be:"
                            ),
                            Vec::new(),
                        ),
                    )
                }
                PContext::PCaseMatch(0) => (
                    reflow("The 1st pattern in this `case` causing a mismatch:"),
                    compare(
                        expected,
                        "The first pattern is trying to match",
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
                        compare(
                            expected,
                            &format!("The {ith} pattern is trying to match"),
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
                        compare(
                            expected,
                            "It is trying to match",
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
                        compare(
                            expected,
                            &format!("The {ith} pattern is trying to match"),
                            "But all the previous patterns in the list are:",
                            vec![mix_types(
                                "Everything in a list must be the same type of value. This way, we never run into unexpected values partway through a List.map, List.foldl, etc. Read",
                            )],
                        ),
                    )
                }
                PContext::PTail => (
                    reflow("The pattern after (::) is causing issues."),
                    compare(
                        expected,
                        "The pattern after (::) is trying to match",
                        "But it needs to match lists like this:",
                        Vec::new(),
                    ),
                ),
            };
            cx.snippet(*region, Some(pattern_region), problem, details)
        }
    };
    Report::mismatch(pattern_region, doc)
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
