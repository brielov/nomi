//! A program's JavaScript (`Generate.hs`): the dependencies' graph and
//! every module's merged, the roots' `main`s gathered, and the code
//! generated in the mode asked for.

use nomi_base::Compat;
use nomi_codegen_js::debug::{ModuleTypes, Types};
use nomi_codegen_js::generate::generate;
use nomi_codegen_js::mode::{Mode, shorten_field_names};
use nomi_compiler::canonical::Home;
use nomi_compiler::optimized::binary::names;
use nomi_compiler::optimized::{GlobalGraph, LocalGraph, Main};
use nomi_project::ModuleName;

use crate::check::Optimized;

/// How code is generated: `elm make`'s default, `--debug` or
/// `--optimize`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dev,
    Debug,
    Optimize,
}

/// The JavaScript of what `elm make` optimized of the project `package`,
/// with `dependencies`' graph and their modules' types.
pub fn program(
    compat: Compat,
    kind: Kind,
    package: &str,
    dependencies: (GlobalGraph, Types),
    optimized: Optimized,
) -> Vec<u8> {
    let Optimized {
        roots,
        graphs: mut locals,
        interfaces,
    } = optimized;
    let home = |name: &ModuleName| Home {
        package: package.to_owned(),
        module: name.to_string(),
    };
    // `gatherMains`.
    let mut mains: Vec<(Home, Main)> = Vec::new();
    for (name, _) in &roots {
        let main = (locals.iter())
            .find(|(local, _)| local == name)
            .and_then(|(_, graph)| graph.main.clone());
        if let Some(main) = main
            && !mains.iter().any(|(h, _)| *h == home(name))
        {
            mains.push((home(name), main));
        }
    }
    let (graph, mut types) = dependencies;
    // `foldr addLocalGraph`: modules in the release's order of names, the
    // first one's applied last.
    locals.sort_by(|a, b| names(compat, &b.0.to_string(), &a.0.to_string()));
    let graph = locals.into_iter().fold(graph, add_local_graph);
    let mode = match kind {
        Kind::Dev => Mode::Dev { debug: None },
        Kind::Debug => {
            for (name, interface) in &interfaces {
                types.insert(home(name), ModuleTypes::from_interface(interface));
            }
            Mode::Dev { debug: Some(types) }
        }
        Kind::Optimize => Mode::Prod(shorten_field_names(compat, &graph)),
    };
    generate(compat, &mode, &graph, &mains)
}

/// `Opt.addLocalGraph`: left-biased unions, so the module's nodes and
/// field counts replace the graph's.
fn add_local_graph(mut graph: GlobalGraph, (_, local): (ModuleName, LocalGraph)) -> GlobalGraph {
    graph.nodes.extend(local.nodes);
    graph.fields.extend(local.fields);
    graph
}
