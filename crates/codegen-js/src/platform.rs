//! What a program is generated for: where it runs, and the shape of its
//! file. Upstream's is the browser's, an IIFE.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fmt::Write;
use std::path::{Component, Path};

use nomi_base::Platform;

use crate::ffi::Ffi;
use crate::runtime::{self, Override};

/// The kernel modules of the packages bundled with nomi, by short name,
/// in order: each `packages/*/src/Elm/Kernel/*.js`.
const BUNDLED_KERNELS: &[&str] = &[
    "Nomi",
    "NomiFfi",
    "NomiFs",
    "NomiServer",
    "NomiStdio",
    "NomiTest",
];

/// The kernel module that calls the FFI module's functions.
pub const FFI_KERNEL: &str = "NomiFfi";

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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Target {
    pub platform: Platform,
    pub format: Format,
    pub dom: Dom,
    /// The FFI module an ES module imports, if it has one.
    pub ffi: Option<Ffi>,
}

/// The specifier a module at `output` imports the file `module` by: the
/// path from `output`'s directory, `./` or `../` first, with forward
/// slashes, and each byte but a letter, digit, `-`, `.`, `_` or `~`
/// percent-encoded, so neither the URL it resolves to nor the string it
/// is written in can end early. Both paths are absolute, or relative to
/// one directory that `output` is not above; `.` and `..` are resolved
/// without the file system.
pub fn ffi_specifier(output: &Path, module: &Path) -> String {
    let from = lexical(output.parent().unwrap_or_else(|| Path::new("")));
    let to = lexical(module);
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts = match from.len() - common {
        0 => vec![".".to_owned()],
        up => vec!["..".to_owned(); up],
    };
    for part in &to[common..] {
        let mut encoded = String::new();
        for byte in part.as_encoded_bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                    encoded.push(char::from(*byte));
                }
                _ => _ = write!(encoded, "%{byte:02X}"),
            }
        }
        parts.push(encoded);
    }
    parts.join("/")
}

/// `path`'s components with `.` dropped and each `..` taking off the one
/// before it, where there is one.
fn lexical(path: &Path) -> Vec<&OsStr> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir if parts.last().is_some_and(|last| *last != "..") => {
                parts.pop();
            }
            other => parts.push(other.as_os_str()),
        }
    }
    parts
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
pub fn probe<'a>(kernels: impl IntoIterator<Item = &'a str>) -> String {
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
/// flags. A
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
    fn an_ffi_module_is_imported_from_the_outputs_directory() {
        let specifier =
            |output: &str, module: &str| ffi_specifier(Path::new(output), Path::new(module));
        assert_eq!(specifier("dist/main.mjs", "dist/ffi.mjs"), "./ffi.mjs");
        assert_eq!(specifier("dist/main.mjs", "src/ffi.mjs"), "../src/ffi.mjs");
        assert_eq!(specifier("main.mjs", "src/js/ffi.mjs"), "./src/js/ffi.mjs");
        assert_eq!(specifier("a/b/c/main.mjs", "a/ffi.mjs"), "../../ffi.mjs");
        assert_eq!(
            specifier("./dist/../out/main.mjs", "out/./ffi.mjs"),
            "./ffi.mjs"
        );
        assert_eq!(
            specifier("/p/dist/main.mjs", "/p/src/ffi.mjs"),
            "../src/ffi.mjs"
        );
        assert_eq!(specifier("dist/main.mjs", "../ffi.mjs"), "../../ffi.mjs");
        assert_eq!(
            specifier("main.mjs", "my lib/it's #1?%.mjs"),
            "./my%20lib/it%27s%20%231%3F%25.mjs"
        );
        assert_eq!(specifier("main.mjs", "é.mjs"), "./%C3%A9.mjs");
    }

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
