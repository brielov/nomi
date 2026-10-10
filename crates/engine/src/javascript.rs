//! A program's JavaScript (`Generate.hs`): the dependencies' graph and
//! every module's merged, the roots' `main`s gathered, and the code
//! generated in the mode asked for.

use nomi_base::Compat;
use nomi_codegen_js::generate::generate;
use nomi_codegen_js::mode::{Mode, shorten_field_names};
use nomi_compiler::canonical::Home;
use nomi_compiler::optimized::{GlobalGraph, LocalGraph, Main};
use nomi_project::ModuleName;

/// How code is generated: `elm make`'s default, `--debug` or
/// `--optimize`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dev,
    Debug,
    Optimize,
}

/// The JavaScript of `roots`, the modules of the project `package` given
/// to `elm make` and whether each defines `main`, from `dependencies`
/// and each module's graph.
pub fn program(
    compat: Compat,
    kind: Kind,
    package: &str,
    dependencies: GlobalGraph,
    roots: &[(ModuleName, bool)],
    locals: Vec<(ModuleName, LocalGraph)>,
) -> Vec<u8> {
    let mut mains: Vec<(Home, Main)> = Vec::new();
    for (name, _) in roots {
        let main = (locals.iter())
            .find(|(local, _)| local == name)
            .and_then(|(_, graph)| graph.main.clone());
        if let Some(main) = main {
            let home = Home {
                package: package.to_owned(),
                module: name.to_string(),
            };
            if !mains.iter().any(|(h, _)| *h == home) {
                mains.push((home, main));
            }
        }
    }
    let graph = locals.into_iter().fold(dependencies, add_local_graph);
    let mode = match kind {
        Kind::Dev => Mode::Dev { debug: false },
        Kind::Debug => Mode::Dev { debug: true },
        Kind::Optimize => Mode::Prod(shorten_field_names(compat, &graph)),
    };
    generate(compat, &mode, &graph, &mains)
}

/// `Opt.addLocalGraph`: the module's nodes over the graph's, and the
/// fields' counts summed.
fn add_local_graph(mut graph: GlobalGraph, (_, local): (ModuleName, LocalGraph)) -> GlobalGraph {
    for (global, node) in local.nodes {
        graph.nodes.insert(global, node);
    }
    for (field, count) in local.fields {
        *graph.fields.entry(field).or_default() += count;
    }
    graph
}
