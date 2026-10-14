//! The optimized graphs each elm wrote for the optimizer cases (`cargo
//! xtask oracle optimized`) decode, and encode back to the same bytes:
//! every module's `.elmo`, and the dependencies' `o.dat`, which holds all
//! of elm/core and elm/json, kernels included.

use std::fs;

use nomi_base::Compat;
use nomi_compiler::optimized::binary;
use nomi_testkit::optimized_artifacts;

#[test]
fn module_graphs_round_trip() {
    let mut checked = 0;
    for compat in Compat::ALL {
        for case in fs::read_dir(optimized_artifacts(compat)).unwrap() {
            let case = case.unwrap().path();
            if !case.is_dir() {
                continue;
            }
            for file in fs::read_dir(&case).unwrap() {
                let file = file.unwrap().path();
                let bytes = fs::read(&file).unwrap();
                let graph = binary::decode_local_graph(&bytes, compat)
                    .unwrap_or_else(|| panic!("{} doesn't decode", file.display()));
                let encoded = binary::encode_local_graph(&graph, compat);
                assert!(encoded == bytes, "{} encodes otherwise", file.display());
                checked += 1;
            }
        }
    }
    assert!(checked >= 12, "only {checked} module graphs");
}

#[test]
fn dependency_graphs_round_trip() {
    for compat in Compat::ALL {
        let path = optimized_artifacts(compat).join("o.dat");
        let bytes = fs::read(&path).unwrap();
        let graph = binary::decode_global_graph(&bytes, compat)
            .unwrap_or_else(|| panic!("{} doesn't decode", path.display()));
        assert!(
            graph.nodes.len() > 400,
            "{}: {} nodes",
            path.display(),
            graph.nodes.len()
        );
        let encoded = binary::encode_global_graph(&graph, compat);
        assert!(encoded == bytes, "{} encodes otherwise", path.display());
    }
}
