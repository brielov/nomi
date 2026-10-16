//! A program's JavaScript (`Generate.hs`): the dependencies' graph and
//! every module's merged, the platform's kernels in place of upstream's,
//! the roots' `main`s gathered, and the code generated in the mode asked
//! for.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use nomi_base::diagnostic::Report;
use nomi_base::diagnostic::doc::{Color, reflow, text};
use nomi_base::{Compat, Platform, kernel};
use nomi_codegen_js::debug::{ModuleTypes, Types};
use nomi_codegen_js::ffi::{self, Exports, Misspelt, Startup};
use nomi_codegen_js::generate::generate;
use nomi_codegen_js::kernel::{add_kernel, parse};
use nomi_codegen_js::mode::{Mode, shorten_field_names};
use nomi_codegen_js::platform;
use nomi_codegen_js::reach::unavailable;
use nomi_compiler::canonical::{Home, Type};
use nomi_compiler::nitpick::debug::graph_has_debug_uses;
use nomi_compiler::optimized::{Global, GlobalGraph, Main};
use nomi_project::{ModuleName, Name};

use crate::Pool;
use crate::artifacts::Hash;
use crate::check::Optimized;
use crate::graph::module_path;
use crate::make::{FfiModule, NamedIn};
use crate::paths::package_src;
use crate::project::package_outline;

pub use nomi_codegen_js::platform::{Dom, Format, Target};

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
/// with `dependencies`' graph and their modules' types, for `target`; or
/// the report on a kernel the program reaches that its platform lacks.
/// `packages` are the dependencies, each with the directory of its
/// source, where the platform's kernels find the files they replace;
/// `ffi`, where `target` has an FFI module, is where it was named.
#[expect(
    clippy::too_many_arguments,
    reason = "each is a separate input of generation"
)]
pub fn program(
    compat: Compat,
    kind: Kind,
    target: &Target,
    ffi: Option<&FfiModule>,
    package: &str,
    packages: &[(Name, PathBuf)],
    dependencies: (GlobalGraph, Types),
    optimized: Optimized,
) -> Result<Vec<u8>, Report> {
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
    let unknown = replace_kernels(compat, target.platform, packages, &mut graph);
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
    let mut lacking = platform::unavailable(target.platform, target.dom).to_vec();
    lacking.extend(unknown.iter().map(|release| release.module));
    if target.platform == Platform::Node && target.ffi.is_none() {
        lacking.push(platform::FFI_KERNEL);
    }
    let js = match unavailable(compat, &mode, &lacking, &graph, &mains) {
        Some(path) => Err(not_available(target.platform, &path, &unknown)),
        None => match &target.ffi {
            Some(module) => ffi::check(compat, &mode, &module.exports, &graph, &mains)
                .map_err(|misspelt| ffi_misspelt(&misspelt, &module.exports, ffi)),
            None => Ok(Startup::default()),
        }
        .map(|startup| generate(compat, &mode, target, &startup, &graph, &mains)),
    };
    // Every package's code is in the graph, so freeing it takes a while,
    // which a build need not wait for.
    Pool::global().drop_later((graph, mode));
    js
}

/// A kernel module the platform replaces, in a release of its package
/// whose file nomi doesn't know.
struct Unknown {
    module: &'static str,
    /// The package and its version, as `elm/core 1.0.5`.
    release: String,
}

/// Puts in `graph`, in place of each kernel module of `packages` that
/// `platform` replaces, the platform's own, keyed by the upstream file it
/// replaces; returns the modules whose file is none it knows, which stay
/// as they are.
fn replace_kernels(
    compat: Compat,
    platform: Platform,
    packages: &[(Name, PathBuf)],
    graph: &mut GlobalGraph,
) -> Vec<Unknown> {
    let mut unknown = Vec::new();
    for replacement in platform::overrides(platform) {
        let Some((name, dir)) =
            (packages.iter()).find(|(name, _)| name.to_string() == replacement.package)
        else {
            continue;
        };
        if !(graph.nodes).contains_key(&Global::kernel(compat, replacement.module)) {
            continue;
        }
        let file = module_path(
            &format!("{}{}", kernel::MODULE_PREFIX, replacement.module),
            "js",
        );
        let digest =
            fs::read(package_src(dir).join(file)).map(|bytes| nomi_base::hex(&Hash::of(&bytes).0));
        let outline = package_outline(dir, compat).ok().flatten();
        if !digest.is_ok_and(|digest| replacement.upstream.contains(&digest.as_str())) {
            let release = match &outline {
                Some(outline) => format!("{name} {}", outline.version),
                None => name.to_string(),
            };
            unknown.push(Unknown {
                module: replacement.module,
                release,
            });
            continue;
        }
        // The modules its dependencies expose, as the package's own kernels
        // read them.
        let mut foreigns = BTreeMap::new();
        for dep in outline.iter().flat_map(|outline| outline.deps.keys()) {
            let exposed = (packages.iter())
                .find(|(name, _)| name == dep)
                .and_then(|(_, dir)| package_outline(dir, compat).ok().flatten());
            for module in exposed.iter().flat_map(|outline| outline.exposed.modules()) {
                foreigns.insert(module.to_string(), dep.to_string());
            }
        }
        let content = parse(replacement.package, &foreigns, replacement.code.as_bytes())
            .expect("a platform's kernels parse");
        add_kernel(compat, graph, replacement.module, content.chunks);
    }
    unknown
}

/// That `path`, from a `main` to a kernel, reaches what `platform` lacks,
/// a kernel it replaces in a release in `unknown`, or, on Node, the FFI's
/// kernel without an FFI module.
fn not_available(platform: Platform, path: &[Global], unknown: &[Unknown]) -> Report {
    let name = |global: &Global| match global.home.package.as_str() {
        "elm/kernel" => format!("{}{}", kernel::MODULE_PREFIX, global.home.module),
        _ => format!("{}.{}", global.home.module, global.name),
    };
    let (main, rest) = path.split_first().expect("a path starts at a main");
    let last = path.last().unwrap_or(main);
    let kernel = name(last);
    let chain = std::iter::once(main.name.clone())
        .chain(rest.iter().map(name))
        .collect::<Vec<_>>()
        .join(" → ");
    if let Some(Unknown { release, .. }) = (unknown.iter()).find(|u| *u.module == *last.home.module)
    {
        return Report::doc(
            "NOT AVAILABLE ON NODE",
            None,
            reflow(&format!(
                "The `main` in {} uses {kernel} of {release}, a file nomi has no Node version of:",
                main.home.module
            )),
            [
                text(&chain).color(Color::Red).indent(4),
                reflow(&format!(
                    "On Node, nomi puts its own version in place of this kernel, made for the \
                     releases of the package it knows, and {release} is not one of them. Depend \
                     on another release, or use a nomi that knows this one."
                )),
            ],
        );
    }
    if platform == Platform::Node && *last.home.module == *platform::FFI_KERNEL {
        return Report::doc(
            "NO FFI MODULE",
            None,
            reflow(&format!(
                "The `main` in {} calls JavaScript through Nomi.Ffi, but this build has no FFI \
                 module to call:",
                main.home.module
            )),
            [
                text(&chain).color(Color::Red).indent(4),
                reflow(
                    "Give the ES module that exports the functions with --ffi, and build to a \
                     file ending in .mjs, which imports it.",
                ),
            ],
        );
    }
    let (title, host, advice) = match platform {
        Platform::Node => (
            "NOT AVAILABLE ON NODE",
            "Node",
            "Node has no DOM, so pages, browser programs and files a user picks only work in a \
             browser. A module may still define them, as long as `main` never uses them.",
        ),
        Platform::Browser => (
            "NOT AVAILABLE IN THE BROWSER",
            "the browser",
            "nomi's own packages need Node. Build with --platform=node to run this program there.",
        ),
    };
    Report::doc(
        title,
        None,
        reflow(&format!(
            "The `main` in {} uses {kernel}, which {host} does not have:",
            main.home.module
        )),
        [text(&chain).color(Color::Red).indent(4), reflow(advice)],
    )
}

/// That calls in `misspelt` name functions the FFI module, which
/// exports `exports` and was named as `named` says, does not export.
fn ffi_misspelt(misspelt: &[Misspelt], exports: &Exports, named: Option<&FfiModule>) -> Report {
    let calls = (misspelt.iter())
        .map(|Misspelt { caller, name }| {
            format!(
                "{}.{} calls Nomi.Ffi.task \"{name}\"",
                caller.home.module, caller.name
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut docs = vec![text(&calls).color(Color::Red).indent(4)];
    if exports.names.is_empty() {
        docs.push(reflow("It exports nothing."));
    } else {
        let names = exports.names.iter().map(String::as_str).collect::<Vec<_>>();
        docs.push(reflow("It exports:"));
        docs.push(text(names.join(", ")).color(Color::Green).indent(4));
    }
    let module = match named {
        Some(FfiModule {
            path,
            named_in: NamedIn::Entry(main),
        }) => format!(
            "{}, the FFI module nomi.toml gives the entry {main}",
            path.display()
        ),
        Some(FfiModule {
            named_in: NamedIn::Flag,
            ..
        })
        | None => "the module given with --ffi".to_owned(),
    };
    docs.push(reflow(&format!(
        "Check the name against {module}; a call names an export exactly, case included."
    )));
    Report::doc(
        "UNKNOWN FFI FUNCTION",
        None,
        reflow(if misspelt.len() == 1 {
            "This call names a function the FFI module does not export:"
        } else {
            "These calls name functions the FFI module does not export:"
        }),
        docs,
    )
}

/// Whether every root's `main` is annotated nomi/core's `Program`, read
/// from the root's interface, so it runs on Node.
pub fn runs_on_node(optimized: &Optimized) -> bool {
    let is_program = |name: &ModuleName| {
        (optimized.interfaces.iter())
            .find(|module| module.name == *name)
            .and_then(|module| module.interface.get().values.get("main"))
            .is_some_and(|main| match &*main.ty {
                Type::Alias { home, name, .. } => {
                    home.package.as_str() == "nomi/core"
                        && home.module.as_str() == "Nomi"
                        && name == "Program"
                }
                _ => false,
            })
    };
    let mut mains = (optimized.roots.iter()).filter(|root| root.has_main);
    mains.clone().next().is_some() && mains.all(|root| is_program(&root.name))
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
