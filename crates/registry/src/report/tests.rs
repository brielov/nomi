use nomi_base::diagnostic::render::{Styles, render};

use super::*;

fn plain(report: &Report, compat: Compat) -> String {
    render(&report.to_doc(), compat, 80, Styles::Plain)
}

#[test]
fn only_0_19_1_suggests_the_reactor() {
    let report = |compat| {
        plain(
            &details_problem(
                &DetailsProblem::NoSolution,
                compat,
                "elm install",
                Some("elm reactor"),
            ),
            compat,
        )
    };
    assert_eq!(
        report(Compat::V0_19_1),
        "\
-- INCOMPATIBLE DEPENDENCIES ------------------------------------------ elm.json

The dependencies in your elm.json are not compatible.

Did you change them by hand? Try to change it back! It is much more reliable to
add dependencies with elm install or the dependency management tool in
elm reactor.

Please ask for help on the community forums if you try those paths and are still
having problems!

"
    );
    assert_eq!(
        report(Compat::V0_19_2),
        "\
-- INCOMPATIBLE DEPENDENCIES ------------------------------------------ elm.json

The dependencies in your elm.json are not compatible.

Did you change them by hand? Try to change it back! It is much more reliable to
add dependencies with elm install.

Please ask for help on the community forums if you try those paths and are still
having problems!

"
    );
}

#[test]
fn without_a_reactor_none_is_suggested() {
    let problem = DetailsProblem::HandEdited;
    assert_eq!(
        plain(
            &details_problem(&problem, Compat::V0_19_1, "nomi install", None),
            Compat::V0_19_1
        ),
        "\
-- ERROR IN DEPENDENCIES ---------------------------------------------- elm.json

It looks like the dependencies elm.json in were edited by hand (or by a 3rd
party tool) leaving them in an invalid state.

Try to change them back to what they were before! It is much more reliable to
add dependencies with nomi install.

Please ask for help on the community forums if you try those paths and are still
having problems!

"
    );
}
