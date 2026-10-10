//! Every kernel module in the dependencies' graph each elm wrote for the
//! optimizer cases (`cargo xtask oracle optimized`) is what nomi parses
//! from the vendored package's file: the same chunks, needing the same
//! globals.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use nomi_base::Compat;
use nomi_codegen_js::kernel;
use nomi_compiler::optimized::{GlobalGraph, Node, binary};

const RELEASES: [(&str, Compat); 3] = [
    ("elm-0.19.1", Compat::V0_19_1),
    ("elm-0.19.2", Compat::V0_19_2),
    ("elm-0.19.3", Compat::V0_19_3),
];

fn packages() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/packages/elm")
}

/// The package directory holding the kernel `short`, and its name.
fn kernel_file(short: &str) -> (String, PathBuf) {
    for project in ["core", "json"] {
        let dir = fs::read_dir(packages().join(project))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let file = dir.join("src/Elm/Kernel").join(format!("{short}.js"));
        if file.exists() {
            return (format!("elm/{project}"), file);
        }
    }
    panic!("no kernel {short} among the vendored packages");
}

/// The modules elm/core exposes, which elm/json's kernel imports from.
fn core_exposed() -> BTreeMap<String, String> {
    let dir = fs::read_dir(packages().join("core"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let text = fs::read_to_string(dir.join("elm.json")).unwrap();
    // Grouped by heading, up to the key after it.
    let start = text.find("\"exposed-modules\"").unwrap() + "\"exposed-modules\"".len();
    let list = &text[start..text[start..].find("\"elm-version\"").unwrap() + start];
    list.split('"')
        .filter(|s| s.chars().next().is_some_and(char::is_uppercase))
        .map(|m| (m.to_owned(), "elm/core".to_owned()))
        .collect()
}

#[test]
fn kernels_parse_as_elm_parsed_them() {
    let foreigns = core_exposed();
    let mut checked = 0;
    for (elm, compat) in RELEASES {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../compiler/tests/optimized/artifacts")
            .join(elm)
            .join("o.dat");
        let graph = binary::decode_global_graph(&fs::read(&path).unwrap(), compat).unwrap();
        for (global, node) in &graph.nodes {
            let Node::Kernel(..) = node else { continue };
            let short = &global.home.module;
            let (package, file) = kernel_file(short);
            let none = BTreeMap::new();
            let foreigns = if package == "elm/core" {
                &none
            } else {
                &foreigns
            };
            let content = kernel::parse(&package, foreigns, &fs::read(&file).unwrap())
                .unwrap_or_else(|| panic!("{} doesn't parse", file.display()));
            let mut ours = GlobalGraph::default();
            kernel::add_kernel(compat, &mut ours, short, content.chunks);
            let Some(Node::Kernel(our_chunks, our_deps)) = ours.nodes.get(global) else {
                panic!("{short} as {elm}: no kernel node");
            };
            let Node::Kernel(chunks, deps) = node else {
                unreachable!()
            };
            if let Some(i) = (0..chunks.len().max(our_chunks.len()))
                .find(|&i| chunks.get(i) != our_chunks.get(i))
            {
                let show = |c: Option<&nomi_compiler::optimized::Chunk>| match c {
                    Some(nomi_compiler::optimized::Chunk::Js(b)) => {
                        format!("Js({:?})", String::from_utf8_lossy(b))
                    }
                    other => format!("{other:?}"),
                };
                panic!(
                    "{short} as {elm}, chunk {i} of {}:\nnomi: {}\nelm:  {}",
                    chunks.len(),
                    show(our_chunks.get(i)),
                    show(chunks.get(i))
                );
            }
            assert_eq!(our_deps, deps, "{short} as {elm}");
            checked += 1;
        }
    }
    assert!(checked >= 30, "only {checked} kernels");
}
