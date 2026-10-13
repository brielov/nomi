//! `Reporting/Error/Docs.hs`: what elm reports of documentation it can't
//! make, a report per problem.

use nomi_base::Compat;
use nomi_base::TextRange;
use nomi_base::diagnostic::Problem;
use nomi_base::diagnostic::code::{self, Source};
use nomi_base::diagnostic::doc::{self, Doc, reflow, snippet, stack, text};

use super::{DefProblem, Error, NameProblem};

/// `toReports`.
pub fn to_problems(error: &Error, source: &Source<'_>, compat: Compat) -> Vec<Problem> {
    let problem = |title: &'static str, region: TextRange, message: Doc| {
        Problem::new(
            title,
            source.region(region, compat),
            code::expand(&message, source, compat),
        )
    };
    let snip = |region: TextRange, before: Doc, after: Doc| snippet(before, region, None, after);
    let more_advice = || {
        doc::link(
            "Note",
            "Read",
            "docs",
            "for more advice on writing great docs. There are a couple important tricks!",
        )
    };
    match error {
        Error::NoDocs(region) => vec![problem(
            "NO DOCS",
            *region,
            snip(
                *region,
                reflow(
                    "You must have a documentation comment between the module declaration and the imports.",
                ),
                reflow("Learn more at <https://package.elm-lang.org/help/documentation-format>"),
            ),
        )],
        Error::ImplicitExposing(region) => vec![problem(
            "IMPLICIT EXPOSING",
            *region,
            snip(
                *region,
                reflow("I need you to be explicit about what this module exposes:"),
                reflow(
                    "A great API usually hides some implementation details, so it is rare that everything in the file should be exposed. And requiring package authors to be explicit about this is a way of adding another quality check before code gets published. So as you write out the public API, ask yourself if it will be easy to understand as people read the documentation!",
                ),
            ),
        )],
        Error::SyntaxProblem(syntax) => {
            vec![nomi_syntax::elm::docs_problem(source, syntax, compat)]
        }
        Error::NameProblems(problems) => (problems.iter())
            .map(|p| match p {
                NameProblem::Duplicate {
                    name,
                    first,
                    second,
                } => problem(
                    "DUPLICATE DOCS",
                    *second,
                    doc::pair(
                        *first,
                        *second,
                        doc::OneLine {
                            before: reflow(&format!(
                                "There can only be one `{name}` in your module documentation, but it is listed twice:"
                            )),
                            after: text("Remove one of them!"),
                        },
                        doc::TwoSnippets {
                            before: reflow(&format!(
                                "There can only be one `{name}` in your module documentation, but I see two. One here:"
                            )),
                            between: text("And another one over here:"),
                            after: text("Remove one of them!"),
                        },
                    ),
                ),
                NameProblem::OnlyInDocs(name, region) => problem(
                    "DOCS MISTAKE",
                    *region,
                    snip(
                        *region,
                        reflow(&format!(
                            "I do not see `{name}` in the `exposing` list, but it is in your module documentation:"
                        )),
                        reflow(&format!(
                            "Does it need to be added to the `exposing` list as well? Or maybe you removed `{name}` and forgot to delete it here?"
                        )),
                    ),
                ),
                NameProblem::OnlyInExports(name, region) => problem(
                    "DOCS MISTAKE",
                    *region,
                    snip(
                        *region,
                        reflow(&format!(
                            "I do not see `{name}` in your module documentation, but it is in your `exposing` list:"
                        )),
                        stack([
                            reflow(&format!(
                                "Add a line like `@docs {name}` to your module documentation!"
                            )),
                            doc::link(
                                "Note",
                                "See",
                                "docs",
                                "for more guidance on writing high quality docs.",
                            ),
                        ]),
                    ),
                ),
            })
            .collect(),
        Error::DefProblems(problems) => (problems.iter())
            .map(|p| match p {
                DefProblem::NoComment(name, region) => problem(
                    "NO DOCS",
                    *region,
                    snip(
                        *region,
                        reflow(&format!(
                            "The `{name}` definition does not have a documentation comment."
                        )),
                        stack([
                            reflow("Add documentation with nice examples of how to use it!"),
                            more_advice(),
                        ]),
                    ),
                ),
                DefProblem::NoAnnotation(name, region) => problem(
                    "NO TYPE ANNOTATION",
                    *region,
                    snip(
                        *region,
                        reflow(&format!(
                            "The `{name}` definition does not have a type annotation."
                        )),
                        stack([
                            reflow(
                                "I use the type variable names from your annotations when generating docs. So if you say `Html msg` in your type annotation, I can use `msg` in the docs and make them a bit clearer. So add an annotation and try to use nice type variables!",
                            ),
                            more_advice(),
                        ]),
                    ),
                ),
            })
            .collect(),
    }
}
