//! What a program is generated for: where it runs, and the shape of its
//! file. Upstream's is the browser's, an IIFE.

use std::collections::BTreeSet;

use nomi_base::Platform;

use crate::runtime::{self, Override};

/// The kernel modules of the packages bundled with nomi, by short name,
/// in order: each `packages/*/src/Elm/Kernel/*.js`.
const BUNDLED_KERNELS: &[&str] = &["Nomi", "NomiFs", "NomiServer", "NomiStdio", "NomiTest"];

/// The kernel modules, by short name, a program for `platform` may not
/// reach, with `dom`. Node has no DOM of its own. The browser lacks
/// nomi/* kernels, which the graph names by short name alone, as it names
/// every kernel.
pub fn unavailable(platform: Platform, dom: Dom) -> &'static [&'static str] {
    match (platform, dom) {
        (Platform::Browser, Dom::Native | Dom::Polyfilled) => BUNDLED_KERNELS,
        (Platform::Node, Dom::Polyfilled) => &[],
        (Platform::Node, Dom::Native) => &["Browser", "VirtualDom", "File"],
    }
}

/// The kernel modules `platform` replaces with its own.
pub fn overrides(platform: Platform) -> &'static [Override] {
    match platform {
        Platform::Browser => &[],
        Platform::Node => &runtime::NODE,
    }
}

/// The shape of a program's file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Format {
    /// A function called on `this`, which it adds `Elm` to: upstream's.
    #[default]
    Iife,
    /// An ES module exporting `Elm`; a module's `this` is undefined.
    EsModule,
}

/// Whether a program starts itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Start {
    /// Its host imports it and starts what it exports: upstream's.
    #[default]
    Library,
    /// It is run as a program: on Node, an ES module of one `main` starts
    /// it, and handles the process's signals.
    Entry,
}

/// Whether a program built for development advises optimizing it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Advice {
    /// It warns once it loads: upstream's.
    #[default]
    Given,
    /// It doesn't, as a program restarted on each change would repeat it.
    Omitted,
}

/// Where a program finds the browser's globals its DOM kernels use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dom {
    /// In its platform, which Node lacks.
    #[default]
    Native,
    /// In stand-ins its host defines, as elm-test's runner does on Node.
    Polyfilled,
}

/// What a program is generated for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Target {
    pub platform: Platform,
    pub format: Format,
    pub dom: Dom,
    pub start: Start,
    pub advice: Advice,
}

/// What the runtime needs from Node beyond the language, as
/// `runtime/src/probe.js`'s `SINCE` names it.
const RUNTIME_NEEDS: &[&str] = &["AbortController", "setImmediate"];

/// What the kernel module `kernel`, by short name, needs from Node beyond
/// the language and the runtime, as `runtime/src/probe.js`'s `SINCE`
/// names it.
fn kernel_needs(kernel: &str) -> &'static [&'static str] {
    match kernel {
        "Http" => &["fetch", "TextDecoder"],
        "NomiFs" => &["process.getBuiltinModule", "TextEncoder", "TextDecoder"],
        "NomiServer" => &["process.getBuiltinModule", "URLSearchParams"],
        _ => &[],
    }
}

/// The line that starts a Node entry, after the runtime and before the
/// kernels, which may use what they need as they load: the runtime's
/// probe of what it and `kernels`, by short name, need from Node, which
/// exits naming the first Node lacks.
pub(crate) fn probe<'a>(kernels: impl IntoIterator<Item = &'a str>) -> String {
    let mut needs: BTreeSet<&str> = RUNTIME_NEEDS.iter().copied().collect();
    needs.extend(kernels.into_iter().flat_map(kernel_needs));
    let list = needs
        .iter()
        .map(|need| format!("'{need}'"))
        .collect::<Vec<_>>();
    format!("_Runtime.probe([{}]);\n", list.join(", "))
}

/// The lines that end a Node entry's function, where the program's
/// runtime is, and start its `main`, in `module`: one lifecycle on the
/// runtime, which handles signals, then `init`, which passes a worker no
/// flags. A nomi/core program's `init` starts the runtime's `main` itself
/// and marks itself so; any other `main` is a worker, whose lifecycle
/// starts here.
pub(crate) fn shim(module: &str) -> String {
    format!(
        "if (!scope.Elm.{module}.init.program) _Runtime.worker(_Scheduler_runtime);\n\
         scope.Elm.{module}.init();\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_probe_checks_what_the_runtime_and_the_reached_kernels_need_once_each() {
        assert_eq!(
            probe([]),
            "_Runtime.probe(['AbortController', 'setImmediate']);\n"
        );
        assert_eq!(
            probe(["Basics", "NomiServer", "NomiFs"]),
            "_Runtime.probe(['AbortController', 'TextDecoder', 'TextEncoder', \
             'URLSearchParams', 'process.getBuiltinModule', 'setImmediate']);\n"
        );
    }

    #[test]
    fn the_runtimes_probe_knows_every_need() {
        let since = include_str!("../../../runtime/src/probe.js");
        let kernels = ["Http", "NomiFs", "NomiServer"];
        let needs = RUNTIME_NEEDS
            .iter()
            .chain(kernels.iter().flat_map(|k| kernel_needs(k)));
        for need in needs {
            let key = if need.contains('.') {
                format!("  \"{need}\": \"")
            } else {
                format!("  {need}: \"")
            };
            assert!(since.contains(&key), "probe.js's SINCE lacks {need}");
        }
    }

    #[test]
    fn a_shim_starts_the_main_under_its_module_path() {
        assert_eq!(
            shim("Main"),
            "if (!scope.Elm.Main.init.program) _Runtime.worker(_Scheduler_runtime);\n\
             scope.Elm.Main.init();\n"
        );
        assert_eq!(
            shim("Server.Main"),
            "if (!scope.Elm.Server.Main.init.program) _Runtime.worker(_Scheduler_runtime);\n\
             scope.Elm.Server.Main.init();\n"
        );
    }
}
