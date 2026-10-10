//! `--optimize` refusing `Debug` (plan 7.2): the report, as elm 0.19.1
//! printed it for a project whose modules `Main` and `Other` use
//! `Debug`, captured from `elm make src/Main.elm --optimize`.

use nomi_base::Compat;
use nomi_base::diagnostic::render::{Styles, render};
use nomi_engine::report::debug_remnants;
use nomi_project::ModuleName;

const ELM: &str = "-- DEBUG REMNANTS --------------------------------------------------------------

There are uses of the `Debug` module in the following modules:

    Main
    Other

But the --optimize flag only works if all `Debug` functions are removed!

Note: The issue is that --optimize strips out info needed by `Debug` functions.
Here are two examples:

    (1) It shortens record field names. This makes the generated JavaScript is
    smaller, but `Debug.toString` cannot know the real field names anymore.

    (2) Values like `type Height = Height Float` are unboxed. This reduces
    allocation, but it also means that `Debug.toString` cannot tell if it is
    looking at a `Height` or `Float` value.

There are a few other cases like that, and it will be much worse once we start
inlining code. That optimization could move `Debug.log` and `Debug.todo` calls,
resulting in unpredictable behavior. I hope that clarifies why this restriction
exists!

";

#[test]
fn debug_remnants_reads_as_elms() {
    let modules: Vec<ModuleName> = ["Main", "Other"]
        .iter()
        .map(|m| ModuleName::new(m))
        .collect();
    let report = debug_remnants(&modules);
    let text = render(&report.to_doc(), Compat::V0_19_1, 80, Styles::Plain);
    assert_eq!(text, ELM);
}
