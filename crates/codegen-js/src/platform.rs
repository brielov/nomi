//! What a program is generated for: where it runs, and the shape of its
//! file. Upstream's is the browser's, an IIFE.

/// Where a program runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Platform {
    #[default]
    Browser,
    Node,
}

impl Platform {
    /// The kernel modules, by short name, a program for the platform may
    /// not reach. Node has no DOM. The browser lacks nomi/* kernels, which
    /// the graph names by short name alone, as it names every kernel.
    pub fn unavailable(self) -> &'static [&'static str] {
        match self {
            Platform::Browser => &[],
            Platform::Node => &["Browser", "VirtualDom", "File"],
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

/// What a program is generated for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Target {
    pub platform: Platform,
    pub format: Format,
}

/// The lines that end a Node entry's module and start its `main`, in
/// `module`: checks of what the program needs from Node go first, then
/// `init`, which passes a worker no flags.
pub fn shim(module: &str) -> String {
    format!("Elm.{module}.init();\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shim_starts_the_main_under_its_module_path() {
        assert_eq!(shim("Main"), "Elm.Main.init();\n");
        assert_eq!(shim("Server.Main"), "Elm.Server.Main.init();\n");
    }
}
