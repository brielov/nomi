//! `main`'s type (`Optimize/Module.hs`'s `addDefHelp` and `findMain`,
//! with `Reporting/Error/Main.hs`): Html, Svg or a `Program` whose flags
//! can come from JavaScript, and not part of a recursive group.

use std::collections::BTreeMap;

use nomi_base::diagnostic::Problem;
use nomi_base::diagnostic::code::{self, Source};
use nomi_base::diagnostic::doc::{self, Color, Doc, reflow, snippet, stack, text};
use nomi_base::{Compat, TextRange};

use crate::canonical::{Annotation, Decl, MainKind, Type};
use crate::canonicalize::effects::check_payload;
use crate::canonicalize::error::InvalidPayload;
use crate::types::localizer::Localizer;
use crate::types::render::{Context, can_to_doc};

/// `Error`.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    BadType(TextRange, Type),
    BadCycle(TextRange, String, Vec<String>),
    BadFlags(TextRange, Type, InvalidPayload),
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
                    let names = ds.iter().map(|def| def.name().to_owned()).collect();
                    return Some(Error::BadCycle(main.region(), d.name().to_owned(), names));
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
        MainKind::Program { flags, .. } => check_payload(&flags).err().map(|bad| {
            let (bad_type, problem) = *bad;
            Error::BadFlags(region, bad_type, problem)
        }),
        MainKind::Other => Some(Error::BadType(region, ty.clone())),
    }
}

/// `toReport`.
pub fn to_problem(
    error: &Error,
    source: &Source<'_>,
    compat: Compat,
    trim: &dyn Fn(TextRange) -> TextRange,
    localizer: &Localizer,
) -> Problem {
    let snip =
        |region: TextRange, before: Doc, after: Doc| snippet(before, trim(region), None, after);
    let (title, region, message) = match error {
        Error::BadType(region, ty) => (
            "BAD MAIN TYPE",
            *region,
            snip(
                *region,
                text("I cannot handle this type of `main` value:"),
                stack([
                    text("The type of `main` value I am seeing is:"),
                    can_to_doc(localizer, Context::None, ty)
                        .color(Color::DullYellow)
                        .indent(4),
                    reflow(
                        "I only know how to handle Html, Svg, and Programs though. Modify `main` to be one of those types of values!",
                    ),
                ]),
            ),
        ),
        Error::BadCycle(region, name, names) => (
            "BAD MAIN",
            *region,
            snip(
                *region,
                text("A `main` definition cannot be defined in terms of itself."),
                stack([
                    reflow(
                        "It should be a boring value with no recursion. But instead it is involved in this cycle of definitions:",
                    ),
                    doc::cycle(std::iter::once(name.clone()).chain(names.iter().cloned())),
                ]),
            ),
        ),
        Error::BadFlags(region, _, problem) => {
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
            (
                "BAD FLAGS",
                *region,
                snip(
                    *region,
                    reflow(&format!(
                        "Your `main` program wants {a_thing} from JavaScript."
                    )),
                    but,
                ),
            )
        }
    };
    let range = trim(region);
    Problem::new(
        title,
        source.region(range, compat),
        code::expand(&message, source, compat),
    )
}
