//! What a program is generated for: where it runs, and the shape of its
//! file. Upstream's is the browser's, an IIFE.

use nomi_base::Platform;

use crate::runtime::{self, Override};

/// The kernel modules of the packages bundled with nomi, by short name,
/// in order: each `packages/*/src/Elm/Kernel/*.js`.
const BUNDLED_KERNELS: &[&str] = &[
    "Nomi",
    "NomiBytes",
    "NomiCompress",
    "NomiCrypto",
    "NomiFs",
    "NomiHttpClient",
    "NomiHttpServer",
    "NomiLog",
    "NomiNet",
    "NomiPath",
    "NomiProcess",
    "NomiSql",
    "NomiStdio",
    "NomiStream",
    "NomiTest",
    "NomiWebSocket",
];

/// The kernel modules, by short name, a program for `platform` may not
/// reach, with `dom`. Bun has no DOM of its own. The browser lacks
/// nomi/* kernels, which the graph names by short name alone, as it names
/// every kernel.
pub fn unavailable(platform: Platform, dom: Dom) -> &'static [&'static str] {
    match (platform, dom) {
        (Platform::Browser, Dom::Native | Dom::Polyfilled) => BUNDLED_KERNELS,
        (Platform::Bun, Dom::Polyfilled) => &[],
        (Platform::Bun, Dom::Native) => &["Browser", "VirtualDom", "File"],
    }
}

/// The kernel modules `platform` replaces with its own.
pub fn overrides(platform: Platform) -> &'static [Override] {
    match platform {
        Platform::Browser => &[],
        Platform::Bun => &runtime::BUN,
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
    /// It is run as a program: on Bun, an ES module of one `main` starts
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
    /// In its platform, which Bun lacks.
    #[default]
    Native,
    /// In stand-ins its host defines, as elm-test's runner does on Bun.
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

/// The line that starts a Bun entry, after the runtime and before the
/// kernels, which may use Bun's built-ins as they load: the runtime's
/// probe, which exits with a line unless the host is a Bun recent enough.
pub(crate) fn probe() -> &'static str {
    "_Runtime.probe();\n"
}

/// The lines that end a Bun entry's function, where the program's
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
    fn the_runtimes_minimum_bun_is_references_tomls() {
        let probe = include_str!("../../../runtime/src/probe.js");
        let references = include_str!("../../../references.toml");
        let minimum = (references.lines())
            .find_map(|line| line.strip_prefix("bun-minimum = "))
            .expect("references.toml names bun-minimum");
        assert!(
            probe.contains(&format!("export const MINIMUM = {minimum};")),
            "probe.js's MINIMUM isn't references.toml's bun-minimum, {minimum}"
        );
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
