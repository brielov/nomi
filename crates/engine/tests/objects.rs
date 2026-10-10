//! The dependencies' graph of elm/core and elm/json, which the optimizer
//! oracle's last case builds with, encodes to each elm's `o.dat` of it
//! (`cargo xtask oracle optimized`).

use std::fs;
use std::path::Path;

use nomi_base::Compat;
use nomi_compiler::optimized::binary;
use nomi_engine::objects::dependency_graph;
use nomi_project::{Name, json::Cursor};

#[test]
fn core_and_json_merge_as_elm_merges_them() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let vendored = manifest.join("../compiler/tests/packages/elm");
    for compat in [Compat::V0_19_1, Compat::V0_19_2, Compat::V0_19_3] {
        let packages: Vec<_> = ["core", "json"]
            .into_iter()
            .map(|project| {
                let dir = fs::read_dir(vendored.join(project))
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path();
                let full = format!("elm/{project}");
                let name = Name::parse(full.as_bytes(), Cursor::start(compat), compat).unwrap();
                (name, dir)
            })
            .collect();
        let graph = dependency_graph(compat, &packages).unwrap();
        let ours = binary::encode_global_graph(&graph, compat);
        let path = manifest
            .join("../compiler/tests/optimized/artifacts")
            .join(format!("elm-{compat}"))
            .join("o.dat");
        let elm = binary::decode_global_graph(&fs::read(&path).unwrap(), compat).unwrap();
        if ours != fs::read(&path).unwrap() {
            let missing: Vec<_> = elm
                .nodes
                .keys()
                .filter(|g| !graph.nodes.contains_key(*g))
                .map(|g| format!("{}.{}", g.home.module, g.name))
                .take(10)
                .collect();
            let extra: Vec<_> = graph
                .nodes
                .keys()
                .filter(|g| !elm.nodes.contains_key(*g))
                .map(|g| format!("{}.{}", g.home.module, g.name))
                .take(10)
                .collect();
            let differ: Vec<_> = elm
                .nodes
                .iter()
                .filter(|(g, n)| graph.nodes.get(*g).is_some_and(|m| m != *n))
                .map(|(g, _)| format!("{}.{}", g.home.module, g.name))
                .take(10)
                .collect();
            let fields: Vec<_> = elm
                .fields
                .iter()
                .filter(|(f, n)| graph.fields.get(*f) != Some(n))
                .take(10)
                .collect();
            panic!(
                "{compat}: missing {missing:?}, extra {extra:?}, differ {differ:?}, fields {fields:?}"
            );
        }
    }
}
