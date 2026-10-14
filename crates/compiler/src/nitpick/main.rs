//! `main`'s type (`Optimize/Module.hs`'s `addDefHelp` and `findMain`,
//! with `Reporting/Error/Main.hs`): Html, Svg or a `Program` whose flags
//! can come from JavaScript, and not part of a recursive group.

use std::collections::BTreeMap;

use nomi_base::TextRange;
use nomi_base::diagnostic::Problem;
use nomi_base::diagnostic::doc::{self, dull_yellow, reflow, stack, text};

use crate::canonical::{Annotation, Decl, MainKind, Type};
use crate::canonicalize::effects::check_payload;
use crate::canonicalize::error::InvalidPayload;
use crate::report::{Report, ReportContext};
use crate::types::localizer::Localizer;
use crate::types::render::{Context, can_to_doc};

/// `Error`.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    BadType(TextRange, Type),
    /// `main` in a cycle of definitions, `first` the one it starts from.
    BadCycle {
        region: TextRange,
        first: String,
        others: Vec<String>,
    },
    BadFlags(TextRange, InvalidPayload),
}

/// The first problem with `main`, in the order upstream adds the
/// declarations, as its `Result` stops at the first.
pub fn check(decls: &[Decl], annotations: &BTreeMap<String, Annotation>) -> Option<Error> {
    for decl in decls {
        match decl {
            Decl::Declare(def) if def.name() == "main" => {
                return main_type(def.region(), &annotations.get("main")?.ty);
            }
            Decl::Declare(_) => {}
            Decl::DeclareRec(d, ds) => {
                if let Some(main) = std::iter::once(d)
                    .chain(ds)
                    .find(|def| def.name() == "main")
                {
                    return Some(Error::BadCycle {
                        region: main.region(),
                        first: d.name().to_owned(),
                        others: ds.iter().map(|def| def.name().to_owned()).collect(),
                    });
                }
            }
        }
    }
    None
}

/// `addDefHelp`'s check of `main`.
fn main_type(region: TextRange, ty: &Type) -> Option<Error> {
    match ty.main_kind() {
        MainKind::Static => None,
        MainKind::Program { flags, .. } => check_payload(&flags)
            .err()
            .map(|problem| Error::BadFlags(region, problem)),
        MainKind::Other => Some(Error::BadType(region, ty.clone())),
    }
}

/// `toReport`.
pub fn to_problem(error: &Error, cx: &ReportContext<'_>, localizer: &Localizer) -> Problem {
    let report = |title, region: TextRange, before, after| {
        Report::new(title, region, cx.snippet(region, None, before, after))
    };
    cx.problem(&match error {
        Error::BadType(region, ty) => report(
            "BAD MAIN TYPE",
            *region,
            text("I cannot handle this type of `main` value:"),
            stack([
                text("The type of `main` value I am seeing is:"),
                dull_yellow(can_to_doc(localizer, Context::None, ty)).indent(4),
                reflow(
                    "I only know how to handle Html, Svg, and Programs though. Modify `main` to be one of those types of values!",
                ),
            ]),
        ),
        Error::BadCycle {
            region,
            first,
            others,
        } => report(
            "BAD MAIN",
            *region,
            text("A `main` definition cannot be defined in terms of itself."),
            stack([
                reflow(
                    "It should be a boring value with no recursion. But instead it is involved in this cycle of definitions:",
                ),
                doc::cycle(std::iter::once(first.clone()).chain(others.iter().cloned())),
            ]),
        ),
        Error::BadFlags(region, problem) => {
            let (a_thing, but) = match problem {
                InvalidPayload::ExtendedRecord => (
                    "an extended record".to_owned(),
                    reflow("But the exact shape of the record must be known at compile time. No type variables!"),
                ),
                InvalidPayload::Function => (
                    "a function".to_owned(),
                    reflow(
                        "But if I allowed functions from JS, it would be possible to sneak side-effects and runtime exceptions into Elm!",
                    ),
                ),
                InvalidPayload::TypeVariable(name) => (
                    "an unspecified type".to_owned(),
                    reflow(&format!(
                        "But type variables like `{name}` cannot be given as flags. I need to know exactly what type of data I am getting, so I can guarantee that unexpected data cannot sneak in and crash the Elm program."
                    )),
                ),
                InvalidPayload::UnsupportedType(name) => (
                    format!("a `{name}` value"),
                    stack([
                        reflow("I cannot handle that. The types that CAN be in flags include:"),
                        reflow("Ints, Floats, Bools, Strings, Maybes, Lists, Arrays, tuples, records, and JSON values.")
                            .indent(4),
                        reflow(
                            "Since JSON values can flow through, you can use JSON encoders and decoders to allow other types through as well. More advanced users often just do everything with encoders and decoders for more control and better errors.",
                        ),
                    ]),
                ),
            };
            report(
                "BAD FLAGS",
                *region,
                reflow(&format!(
                    "Your `main` program wants {a_thing} from JavaScript."
                )),
                but,
            )
        }
    })
}
