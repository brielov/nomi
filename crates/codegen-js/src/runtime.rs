//! The fiber runtime in a Node program, and the kernels Node replaces:
//! `runtime/src`'s modules, one namespace ahead of the kernels, and
//! `runtime/node/kernel`'s files, each in place of the upstream file it
//! names by hash.

use std::sync::LazyLock;

/// The runtime's modules a program holds, each after those it imports.
const MODULES: [(&str, &str); 3] = [
    ("clock.js", include_str!("../../../runtime/src/clock.js")),
    ("fiber.js", include_str!("../../../runtime/src/fiber.js")),
    ("main.js", include_str!("../../../runtime/src/main.js")),
];

/// What a Node program declares before its kernels: `_Runtime`, the
/// runtime's exports.
pub fn prologue() -> &'static str {
    static PROLOGUE: LazyLock<String> = LazyLock::new(|| {
        namespace("_Runtime", &MODULES).expect("runtime/src uses only the forms `namespace` reads")
    });
    &PROLOGUE
}

/// `modules`, each a file name and an ES module importing only the ones
/// before it, as one script declaring `name`: the modules' code in one
/// function, without their `import` lines and `export` keywords,
/// returning what they export. A form it doesn't read is an error naming
/// the module and the line.
pub fn namespace(name: &str, modules: &[(&str, &str)]) -> Result<String, String> {
    let mut code = String::new();
    let mut exports = Vec::new();
    for (i, (file, text)) in modules.iter().enumerate() {
        let mut lines = text.lines();
        while let Some(line) = lines.next() {
            let bad = || format!("{file}: `{line}` is not a form the runtime may use");
            if line.starts_with("import ") {
                let mut statement = line.to_owned();
                while !statement.ends_with(';') {
                    statement.push_str(lines.next().ok_or_else(bad)?);
                }
                let from = (statement.split_once(" from "))
                    .and_then(|(_, from)| from.strip_suffix(';'))
                    .and_then(|from| from.strip_prefix("\"./")?.strip_suffix('"'));
                if !from.is_some_and(|from| modules[..i].iter().any(|(f, _)| *f == from)) {
                    return Err(bad());
                }
            } else if let Some(rest) = line.strip_prefix("export ") {
                let declared = ["const ", "let ", "class ", "function ", "async function "]
                    .iter()
                    .find_map(|keyword| rest.strip_prefix(keyword))
                    .ok_or_else(bad)?;
                let end = (declared.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')))
                    .unwrap_or(declared.len());
                exports.push(&declared[..end]);
                code.push_str(rest);
                code.push('\n');
            } else {
                code.push_str(line);
                code.push('\n');
            }
        }
    }
    Ok(format!(
        "var {name} = (function() {{\n{code}return {{ {} }};\n}}());\n",
        exports.join(", ")
    ))
}

/// A kernel module a platform replaces, in the package that holds it.
#[derive(Clone, Copy, Debug)]
pub struct Override {
    /// The package, as `elm/core`.
    pub package: &'static str,
    /// The kernel module's short name, as `Scheduler`.
    pub module: &'static str,
    /// The SHA-256 digests, in hex, of the upstream files it replaces.
    pub upstream: &'static [&'static str],
    /// The replacement, a kernel module's text.
    pub code: &'static str,
}

/// The kernels a Node program replaces.
pub(crate) const NODE: [Override; 2] = [
    Override {
        package: "elm/core",
        module: "Scheduler",
        // elm/core 1.0.0 to 1.0.5, byte for byte.
        upstream: &["bdc18efab541e63edd65173fd995e35edc0fefe182fdb46c914b56734e2dbe6a"],
        code: include_str!("../../../runtime/node/kernel/elm/core/Scheduler.js"),
    },
    Override {
        package: "elm/core",
        module: "Process",
        // elm/core 1.0.0 to 1.0.5, byte for byte.
        upstream: &["50554f94246d84921109584196c7c43f90775ff4256ce4df363b44f7794065da"],
        code: include_str!("../../../runtime/node/kernel/elm/core/Process.js"),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_drops_imports_and_returns_exports() {
        let a = "// A.\nexport const one = 1;\nexport class Two {}\n";
        let b = "import { one,\n  Two } from \"./a.js\";\n\nexport function three() {\n  return one + 2;\n}\nconst hidden = 4;\n";
        assert_eq!(
            namespace("_N", &[("a.js", a), ("b.js", b)]).unwrap(),
            "var _N = (function() {\n// A.\nconst one = 1;\nclass Two {}\n\nfunction three() {\n  return one + 2;\n}\nconst hidden = 4;\nreturn { one, Two, three };\n}());\n"
        );
    }

    #[test]
    fn namespace_refuses_what_it_cannot_inline() {
        let refused = [
            "import { format } from \"node:util\";",
            "import { x } from \"./later.js\";",
            "import x from \"./a.js\"",
            "export default 1;",
            "export { x };",
            "export * from \"./a.js\";",
        ];
        for line in refused {
            let modules = [("a.js", "export const a = 1;\n"), ("b.js", line)];
            let error = namespace("_N", &modules).unwrap_err();
            assert!(error.starts_with("b.js: `"), "{line}: {error}");
        }
    }

    #[test]
    fn the_runtime_is_one_namespace() {
        let prologue = prologue();
        assert!(prologue.starts_with("var _Runtime = (function() {\n"));
        assert!(!prologue.contains("\nimport ") && !prologue.contains("\nexport "));
        assert!(prologue.contains("return { RealClock, TestClock, ExitKind, Exit, succeed,"));
    }
}
