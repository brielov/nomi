//! Records whose fields are missing, and types that contain themselves.

use std::collections::BTreeMap;

use super::*;
use crate::types::in_name_order;
use crate::types::render;

/// `toExprReport`'s `RecordAccess`: a field the record lacks.
#[expect(
    clippy::too_many_arguments,
    reason = "the problem's fields and the expression's regions, as `toExprReport` has them"
)]
pub(super) fn record_access(
    cx: &ReportContext<'_>,
    expr_region: TextRange,
    region: TextRange,
    record_region: TextRange,
    name: Option<&str>,
    field_region: TextRange,
    field: &str,
    category: &Category,
    actual: &t::Type,
    expected: &t::Type,
) -> (&'static str, TextRange, Doc) {
    let localizer = cx.localizer;
    let quoted = name.map_or(String::new(), |n| format!("`{n}`"));
    if let t::Type::Record(fields, ext) = t::iterated_dealias(actual.clone()) {
        let details = match nearby(localizer, field, &fields, &ext) {
            None => reflow("In fact, it is a record with NO fields!"),
            Some((nearest, record)) => stack([
                reflow(&format!(
                    "This is usually a typo. Here are the {quoted} fields that are most similar:"
                )),
                record,
                so_maybe(field, &nearest),
            ]),
        };
        let problem = reflow(&format!(
            "This {quoted} record does not have a `{field}` field:"
        ));
        (
            "TYPE MISMATCH",
            expr_region,
            snip(cx, region, Some(field_region), problem, details),
        )
    } else {
        let mut need = words("But I need a record with a");
        need.push(yellow(text(field.to_owned())));
        need.push(text("field!"));
        let lone = lone_type(
            localizer,
            actual,
            expected,
            reflow(&add_category("It is", category)),
            vec![fill_sep(need)],
        );
        let problem = reflow("This is not a record, so it has no fields to access!");
        (
            "TYPE MISMATCH",
            expr_region,
            snip(cx, region, Some(record_region), problem, lone),
        )
    }
}

/// `toExprReport`'s `RecordUpdateKeys`: an update of a field the record
/// lacks.
#[expect(
    clippy::too_many_arguments,
    reason = "the problem's fields and the expression's regions, as `toExprReport` has them"
)]
pub(super) fn record_update_keys(
    cx: &ReportContext<'_>,
    expr_region: TextRange,
    region: TextRange,
    record: &str,
    expected_fields: &BTreeMap<String, TextRange>,
    category: &Category,
    actual: &t::Type,
    expected: &t::Type,
) -> (&'static str, TextRange, Doc) {
    let localizer = cx.localizer;
    if let t::Type::Record(actual_fields, ext) = t::iterated_dealias(actual.clone()) {
        // `Map.lookupMin (Map.difference expected actual)`, in the
        // release's name order.
        let missing = (in_name_order(cx.compat, expected_fields).into_iter())
            .find(|(name, _)| !actual_fields.contains_key(*name));
        match missing {
            None => {
                let comparison = type_comparison(
                    localizer,
                    actual,
                    expected,
                    &add_category(&format!("The `{record}` record is"), category),
                    "But this update needs it to be compatable with:",
                    vec![reflow(
                        "Do you mind creating an <http://sscce.org/> that produces this error message and sharing it at <https://github.com/elm/error-message-catalog/issues> so we can try to give better advice here?",
                    )],
                );
                let problem = reflow("Something is off with this record update:");
                (
                    "TYPE MISMATCH",
                    expr_region,
                    snip(cx, region, None, problem, comparison),
                )
            }
            Some((field, field_region)) => {
                let r = format!("`{record}`");
                let details = match nearby(localizer, field, &actual_fields, &ext) {
                    None => reflow(&format!("In fact, {r} is a record with NO fields!")),
                    Some((nearest, nearby_record)) => stack([
                        reflow(&format!(
                            "This is usually a typo. Here are the {r} fields that are most similar:"
                        )),
                        nearby_record,
                        so_maybe(field, &nearest),
                    ]),
                };
                let problem = reflow(&format!("The {r} record does not have a `{field}` field:"));
                (
                    "TYPE MISMATCH",
                    expr_region,
                    snip(cx, region, Some(*field_region), problem, details),
                )
            }
        }
    } else {
        let lone = lone_type(
            localizer,
            actual,
            expected,
            reflow(&add_category("It is", category)),
            vec![reflow("But I need a record!")],
        );
        let problem = reflow("This is not a record, so it has no fields to update!");
        (
            "TYPE MISMATCH",
            expr_region,
            snip(cx, region, Some(expr_region), problem, lone),
        )
    }
}

/// `So maybe field should be nearest?`.
fn so_maybe(field: &str, nearest: &str) -> Doc {
    let mut docs = words("So maybe");
    docs.push(yellow(text(field.to_owned())));
    docs.extend(words("should be"));
    docs.push(green(text(nearest.to_owned())).append(text("?")));
    fill_sep(docs)
}

/// The fields nearest `field`, by `Suggest.sort` over the record's fields
/// in the release's order, as `toNearbyRecord` shows them; none for a
/// record with no fields.
fn nearby(
    localizer: &Localizer,
    field: &str,
    fields: &BTreeMap<String, t::Type>,
    ext: &t::Extension,
) -> Option<(String, Doc)> {
    let candidates: Vec<String> = in_name_order(localizer.compat, fields)
        .into_iter()
        .map(|(n, _)| n.clone())
        .collect();
    let sorted = nearest(field, candidates);
    let (first, rest) = sorted.split_first()?;
    let doc = |n: &String| {
        (
            text(n.clone()),
            t::to_doc(localizer, render::Context::None, &fields[n]),
        )
    };
    let record = if rest.len() <= 3 {
        let entries = std::iter::once(first).chain(rest).map(doc).collect();
        let ext = match ext {
            t::Extension::Closed => None,
            t::Extension::FlexOpen(x) | t::Extension::RigidOpen(x) => Some(text(x.clone())),
        };
        render::vrecord(entries, ext)
    } else {
        render::vrecord_snippet(doc(first), rest.iter().take(3).map(doc).collect())
    };
    Some((first.clone(), record.indent(4)))
}

/// `toInfiniteReport`.
pub(super) fn infinite(
    cx: &ReportContext<'_>,
    region: TextRange,
    name: &str,
    overall: &t::Type,
) -> (&'static str, TextRange, Doc) {
    let details = stack([
        reflow(
            "Here is my best effort at writing down the type. You will see ∞ for parts of the type that repeat something already printed out infinitely.",
        ),
        yellow(t::to_doc(cx.localizer, render::Context::None, overall)).indent(4),
        reflow_link(
            "Staring at this type is usually not so helpful, so I recommend reading the hints at",
            "infinite-type",
            "to get unstuck!",
        ),
    ]);
    let problem = reflow(&format!(
        "I am inferring a weird self-referential type for {name}:"
    ));
    (
        "INFINITE TYPE",
        region,
        snip(cx, region, None, problem, details),
    )
}
