//! A program's JavaScript (`Generate.hs`): the dependencies' graph and
//! every module's merged, the roots' `main`s gathered, and the code
//! generated in the mode asked for.

use nomi_base::Compat;
use nomi_codegen_js::debug::{ModuleTypes, Types};
use nomi_codegen_js::generate::generate;
use nomi_codegen_js::mode::{Mode, shorten_field_names};
use nomi_compiler::canonical::Home;
use nomi_compiler::nitpick::debug::graph_has_debug_uses;
use nomi_compiler::optimized::{GlobalGraph, Main};
use nomi_project::ModuleName;

use crate::Pool;
use crate::check::Optimized;

pub use nomi_codegen_js::platform::{Format, Platform, Target};

/// How code is generated: `elm make`'s default, `--debug` or
/// `--optimize`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `elm make`'s default.
    Dev,
    /// `--debug`, with the debugger and the types it shows.
    Debug,
    /// `--optimize`, with field names shortened.
    Optimize,
}

/// The JavaScript of what `elm make` optimized of the project `package`,
/// with `dependencies`' graph and their modules' types, for `target`.
pub fn program(
    compat: Compat,
    kind: Kind,
    target: Target,
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
        package: package.into(),
        module: name.to_string().into(),
    };
    // `gatherMains`.
    let mut mains: Vec<(Home, Main)> = Vec::new();
    for root in &roots {
        let main = (locals.iter())
            .find(|(local, _)| *local == root.name)
            .and_then(|(_, graph)| graph.main.clone());
        if let Some(main) = main {
            let home = home(&root.name);
            if !mains.iter().any(|(h, _)| *h == home) {
                mains.push((home, main));
            }
        }
    }
    let (mut graph, mut types) = dependencies;
    // `foldr addLocalGraph`: modules in the release's order of names, the
    // first one's applied last.
    locals.sort_by(|a, b| b.0.compare(&a.0, compat));
    for (_, local) in locals {
        graph.add_local(local);
    }
    let mode = match kind {
        Kind::Dev => Mode::Dev { debug: None },
        Kind::Debug => {
            for module in &interfaces {
                let types_of = ModuleTypes::from_interface(module.interface.get());
                types.insert(home(&module.name), types_of);
            }
            Mode::Dev { debug: Some(types) }
        }
        Kind::Optimize => Mode::Prod(shorten_field_names(compat, &graph)),
    };
    let js = generate(compat, &mode, target, &graph, &mains);
    // Every package's code is in the graph, so freeing it takes a while,
    // which a build need not wait for.
    Pool::global().drop_later((graph, mode));
    js
}

/// `checkForDebugUses`: the modules using `Debug`, in the release's order
/// of names, which `--optimize` refuses (`report::debug_remnants`).
pub fn debug_uses(compat: Compat, optimized: &Optimized) -> Vec<ModuleName> {
    let mut modules: Vec<ModuleName> = (optimized.graphs.iter())
        .filter(|(_, graph)| graph_has_debug_uses(graph))
        .map(|(name, _)| name.clone())
        .collect();
    modules.sort_by(|a, b| a.compare(b, compat));
    modules
}
