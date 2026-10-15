//! Records whose fields are missing, and types that contain themselves.

use std::collections::BTreeMap;

use nomi_base::TextRange;
use nomi_base::diagnostic::doc::{
    Doc, dull_yellow as yellow, fill_sep, green, reflow, reflow_link, stack, text, words,
};
use nomi_base::suggest::sort_texts;

use super::{HintOrder, Mismatch, add_category, lone_type, t, type_comparison};
use crate::report::{Report, ReportContext};
use crate::types::localizer::Localizer;
use crate::types::render;

/// `toExprReport`'s `RecordAccess`: a field the record lacks.
pub(super) fn record_access(
    cx: &ReportContext<'_>,
    localizer: &Localizer,
    m: &Mismatch<'_>,
    record_region: TextRange,
    name: Option<&str>,
    field_region: TextRange,
    field: &str,
) -> Report {
    let quoted = name.map_or(String::new(), |n| format!("`{n}`"));
    if let t::Type::Record(fields, ext) = t::iterated_dealias(m.actual) {
        let details = typo_details(localizer, &quoted, field, fields, ext, || {
            reflow("In fact, it is a record with NO fields!")
        });
        let problem = reflow(&format!(
            "This {quoted} record does not have a `{field}` field:"
        ));
        let doc = cx.snippet(m.region, Some(field_region), problem, details);
        Report::mismatch(m.expr_region, doc)
    } else {
        let need = (words("But I need a record with a")).chain([yellow(field), text("field!")]);
        let lone = lone_type(
            localizer,
            m.actual,
            m.expected,
            reflow(&add_category("It is", m.category)),
            vec![fill_sep(need)],
        );
        let problem = reflow("This is not a record, so it has no fields to access!");
        let doc = cx.snippet(m.region, Some(record_region), problem, lone);
        Report::mismatch(m.expr_region, doc)
    }
}

/// `toExprReport`'s `RecordUpdateKeys`: an update of a field the record
/// lacks.
pub(super) fn record_update_keys(
    cx: &ReportContext<'_>,
    localizer: &Localizer,
    m: &Mismatch<'_>,
    record: &str,
    expected_fields: &BTreeMap<String, TextRange>,
) -> Report {
    let t::Type::Record(actual_fields, ext) = t::iterated_dealias(m.actual) else {
        let lone = lone_type(
            localizer,
            m.actual,
            m.expected,
            reflow(&add_category("It is", m.category)),
            vec![reflow("But I need a record!")],
        );
        let problem = reflow("This is not a record, so it has no fields to update!");
        let doc = cx.snippet(m.region, Some(m.expr_region), problem, lone);
        return Report::mismatch(m.expr_region, doc);
    };
    // `Map.lookupMin (Map.difference expected actual)`, in the release's
    // name order.
    let missing = (localizer.compat.in_name_order(expected_fields).into_iter())
        .find(|(name, _)| !actual_fields.contains_key(*name));
    let doc = match missing {
        None => {
            let comparison = type_comparison(
                localizer,
                m.actual,
                m.expected,
                &add_category(&format!("The `{record}` record is"), m.category),
                "But this update needs it to be compatable with:",
                vec![reflow(
                    "Do you mind creating an <http://sscce.org/> that produces this error message and sharing it at <https://github.com/elm/error-message-catalog/issues> so we can try to give better advice here?",
                )],
                HintOrder::ContextFirst,
            );
            let problem = reflow("Something is off with this record update:");
            cx.snippet(m.region, None, problem, comparison)
        }
        Some((field, field_region)) => {
            let r = format!("`{record}`");
            let details = typo_details(localizer, &r, field, actual_fields, ext, || {
                reflow(&format!("In fact, {r} is a record with NO fields!"))
            });
            let problem = reflow(&format!("The {r} record does not have a `{field}` field:"));
            cx.snippet(m.region, Some(*field_region), problem, details)
        }
    };
    Report::mismatch(m.expr_region, doc)
}

/// What a missing field is likely a typo of: the `quoted` record's fields
/// nearest `field`, and which it may have meant; `no_fields` where the
/// record has none.
fn typo_details(
    localizer: &Localizer,
    quoted: &str,
    field: &str,
    fields: &BTreeMap<String, t::Type>,
    ext: &t::Extension,
    no_fields: impl FnOnce() -> Doc,
) -> Doc {
    // `Suggest.sort` over the record's fields in the release's order, as
    // `toNearbyRecord` shows them.
    let mut sorted: Vec<&String> = localizer
        .compat
        .in_name_order(fields)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    sort_texts(field, &mut sorted);
    let Some((first, rest)) = sorted.split_first() else {
        return no_fields();
    };
    let doc = |n: &&String| {
        (
            text(n.as_str()),
            t::to_doc(localizer, render::Context::None, &fields[*n]),
        )
    };
    let record = if rest.len() <= 3 {
        let entries = std::iter::once(first).chain(rest).map(doc).collect();
        render::vrecord(entries, t::ext_to_doc(ext))
    } else {
        render::vrecord_snippet(doc(first), rest.iter().take(3).map(doc).collect())
    };
    let so_maybe = (words("So maybe"))
        .chain([yellow(field)])
        .chain(words("should be"))
        .chain([green(first.as_str()).append(text("?"))]);
    stack([
        reflow(&format!(
            "This is usually a typo. Here are the {quoted} fields that are most similar:"
        )),
        record.indent(4),
        fill_sep(so_maybe),
    ])
}

/// `toInfiniteReport`.
pub(super) fn infinite(
    cx: &ReportContext<'_>,
    localizer: &Localizer,
    region: TextRange,
    name: &str,
    overall: &t::Type,
) -> Report {
    let details = stack([
        reflow(
            "Here is my best effort at writing down the type. You will see ∞ for parts of the type that repeat something already printed out infinitely.",
        ),
        yellow(t::to_doc(localizer, render::Context::None, overall)).indent(4),
        reflow_link(
            "Staring at this type is usually not so helpful, so I recommend reading the hints at",
            "infinite-type",
            "to get unstuck!",
        ),
    ]);
    let problem = reflow(&format!(
        "I am inferring a weird self-referential type for {name}:"
    ));
    let doc = cx.snippet(region, None, problem, details);
    Report::new("INFINITE TYPE", region, doc)
}
