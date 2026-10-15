//! What a program is generated for: where it runs, and the shape of its
//! file. Upstream's is the browser's, an IIFE.

use crate::runtime::{self, Override};

/// Where a program runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Platform {
    #[default]
    Browser,
    Node,
}

/// The kernel modules of the packages bundled with nomi, by short name,
/// in order: each `packages/*/src/Elm/Kernel/*.js`.
const BUNDLED_KERNELS: &[&str] = &["Nomi", "NomiFs", "NomiServer", "NomiStdio"];

impl Platform {
    /// The kernel modules, by short name, a program for the platform may
    /// not reach, with `dom`. Node has no DOM of its own. The browser
    /// lacks nomi/* kernels, which the graph names by short name alone, as
    /// it names every kernel.
    pub fn unavailable(self, dom: Dom) -> &'static [&'static str] {
        match (self, dom) {
            (Platform::Browser, Dom::Native | Dom::Polyfilled) => BUNDLED_KERNELS,
            (Platform::Node, Dom::Polyfilled) => &[],
            (Platform::Node, Dom::Native) => &["Browser", "VirtualDom", "File"],
        }
    }

    /// The kernel modules the platform replaces with its own.
    pub fn overrides(self) -> &'static [Override] {
        match self {
            Platform::Browser => &[],
            Platform::Node => &runtime::NODE,
        }
    }
}

/// The shape of a program's file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    /// A function called on `this`, which it adds `Elm` to: upstream's.
    #[default]
    Iife,
    /// An ES module exporting `Elm`; a module's `this` is undefined.
    EsModule,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Target {
    pub platform: Platform,
    pub format: Format,
    pub dom: Dom,
}

/// The lines that end a Node entry's function, where the program's
/// runtime is, and start its `main`, in `module`: checks of what the
/// program needs from Node go first, then one lifecycle on the runtime,
/// which handles signals, then `init`, which passes a worker no flags. A
/// nomi/core program's `init` starts the runtime's `main` itself and
/// marks itself so; any other `main` is a worker, whose lifecycle starts
/// here.
pub fn shim(module: &str) -> String {
    format!(
        "if (!scope.Elm.{module}.init.program) _Runtime.worker(_Scheduler_runtime);\n\
         scope.Elm.{module}.init();\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
