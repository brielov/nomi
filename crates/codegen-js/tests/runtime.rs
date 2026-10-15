//! The runtime a Node program carries, and the kernels Node replaces,
//! against the files they come from: `_Runtime` holds what the runtime's
//! modules export, and each kernel replaces the vendored package's file
//! it names, keeping every name the other kernels and Elm modules use.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use nomi_codegen_js::kernel;
use nomi_codegen_js::platform::Platform;
use nomi_codegen_js::runtime::prologue;
use nomi_testkit::{packages, sha256_hex};

/// Each name `_Runtime` holds and what each of the runtime's modules
/// exports under it, imported as Node imports them, with its `typeof`,
/// one per line.
#[test]
fn the_runtime_namespace_holds_what_its_modules_export() {
    nomi_testkit::require_node();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/src");
    let script = r#"
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
const [src] = process.argv.slice(1);
const modules = ["clock.js", "fiber.js", "main.js"];
const exported = {};
for (const file of modules) Object.assign(exported, await import(pathToFileURL(`${src}/${file}`)));
const runtime = new Function(`'use strict';\n${readFileSync(0, "utf8")}return _Runtime;`)();
const names = (o) => Object.keys(o).sort().map((k) => `${k} ${typeof o[k]}`).join("\n");
console.log(names(runtime) === names(exported) ? "same" : `${names(runtime)}\n--\n${names(exported)}`);
"#;
    let mut node = Command::new("node")
        .args(["--input-type=module", "-e", script])
        .arg(&src)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    node.stdin
        .take()
        .unwrap()
        .write_all(prologue().as_bytes())
        .unwrap();
    let ran = node.wait_with_output().unwrap();
    assert!(ran.status.success(), "{ran:?}");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "same\n");
}

/// Every file with `ext` under `dir`, at any depth.
fn files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut dirs = vec![dir.to_owned()];
    while let Some(at) = dirs.pop() {
        for entry in fs::read_dir(&at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == ext) {
                found.push(path);
            }
        }
    }
    found
}

/// The names after each `prefix` in `text`.
fn names_after<'a>(text: &'a str, prefix: &str) -> impl Iterator<Item = &'a str> {
    text.match_indices(prefix).map(move |(at, _)| {
        let rest = &text[at + prefix.len()..];
        let end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        &rest[..end]
    })
}

#[test]
fn each_node_kernel_replaces_the_vendored_file_and_keeps_its_names() {
    for replacement in Platform::Node.overrides() {
        let package = packages::release(replacement.package).dir();
        let module = replacement.module;
        let upstream = package.join(format!("src/Elm/Kernel/{module}.js"));
        let digest = sha256_hex(&fs::read(&upstream).unwrap());
        assert!(
            replacement.upstream.contains(&digest.as_str()),
            "{module}: {digest}"
        );
        assert!(
            kernel::parse(
                replacement.package,
                &Default::default(),
                replacement.code.as_bytes()
            )
            .is_some(),
            "{module} doesn't parse as a kernel"
        );
        // What every other package's kernels and modules use of it, by
        // import or, as elm/time's `_Scheduler_rawSpawn`, by its name.
        let mut used = BTreeSet::new();
        for file in files(&packages::dir(), "js") {
            if file == upstream {
                continue;
            }
            let text = fs::read_to_string(&file).unwrap();
            used.extend(names_after(&text, &format!("__{module}_")).map(str::to_owned));
            used.extend(names_after(&text, &format!(" _{module}_")).map(str::to_owned));
        }
        for file in files(&packages::dir(), "elm") {
            let text = fs::read_to_string(&file).unwrap();
            used.extend(names_after(&text, &format!("Elm.Kernel.{module}.")).map(str::to_owned));
        }
        assert!(!used.is_empty(), "{module}");
        let defined: BTreeSet<&str> = names_after(replacement.code, &format!("_{module}_"))
            .filter(|name| !name.is_empty())
            .collect();
        let missing: Vec<&String> = (used.iter())
            .filter(|name| !defined.contains(name.as_str()))
            .collect();
        assert!(missing.is_empty(), "{module} lacks {missing:?}");
    }
}
