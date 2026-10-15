//! Every kernel module in the dependencies' graph each elm wrote for the
//! optimizer cases (`cargo xtask oracle optimized`) is what nomi parses
//! from the vendored package's file: the same chunks, needing the same
//! globals.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use nomi_base::Compat;
use nomi_codegen_js::kernel;
use nomi_compiler::optimized::{Chunk, GlobalGraph, Node, binary};
use nomi_testkit::packages;

/// The modules elm/core exposes, under its headings, which elm/json's
/// kernel imports from.
fn exposed(core: &Path) -> BTreeMap<String, String> {
    let text = fs::read_to_string(core.join("elm.json")).unwrap();
    let outline: serde_json::Value = serde_json::from_str(&text).unwrap();
    let headings = outline["exposed-modules"].as_object().unwrap();
    (headings.values())
        .flat_map(|modules| modules.as_array().unwrap())
        .map(|module| (module.as_str().unwrap().to_owned(), "elm/core".to_owned()))
        .collect()
}

#[test]
fn kernels_parse_as_elm_parsed_them() {
    let core = packages::release("elm/core").dir();
    let json = packages::release("elm/json").dir();
    let core_exposed = exposed(&core);
    let none = BTreeMap::new();
    let mut checked = 0;
    for compat in Compat::ALL {
        let elm = format!("elm-{compat}");
        let path = nomi_testkit::optimized_artifacts(compat).join("o.dat");
        let graph = binary::decode_global_graph(&fs::read(&path).unwrap(), compat).unwrap();
        for (global, node) in &graph.nodes {
            let Node::Kernel(chunks, deps) = node else {
                continue;
            };
            let short = &global.home.module;
            let file = |dir: &Path| dir.join("src/Elm/Kernel").join(format!("{short}.js"));
            let (package, file, foreigns) = if file(&core).exists() {
                ("elm/core", file(&core), &none)
            } else {
                ("elm/json", file(&json), &core_exposed)
            };
            let content = kernel::parse(package, foreigns, &fs::read(&file).unwrap())
                .unwrap_or_else(|| panic!("{} doesn't parse", file.display()));
            let mut ours = GlobalGraph::default();
            kernel::add_kernel(compat, &mut ours, short, content.chunks);
            let Some(Node::Kernel(our_chunks, our_deps)) = ours.nodes.get(global) else {
                panic!("{short} as {elm}: no kernel node");
            };
            if let Some(i) = (0..chunks.len().max(our_chunks.len()))
                .find(|&i| chunks.get(i) != our_chunks.get(i))
            {
                let show = |c: Option<&Chunk>| match c {
                    Some(Chunk::Js(b)) => format!("Js({:?})", String::from_utf8_lossy(b)),
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
