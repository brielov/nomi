//! The optimized graphs each elm wrote for the optimizer cases (`cargo
//! xtask oracle optimized`) decode, and encode back to the same bytes:
//! every module's `.elmo`, and the dependencies' `o.dat`, which holds all
//! of elm/core and elm/json, kernels included.

use std::fs;
use std::path::{Path, PathBuf};

use nomi_base::Compat;
use nomi_compiler::optimized::binary;

const RELEASES: [(&str, Compat); 3] = [
    ("elm-0.19.1", Compat::V0_19_1),
    ("elm-0.19.2", Compat::V0_19_2),
    ("elm-0.19.3", Compat::V0_19_3),
];

fn artifacts(elm: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/optimized/artifacts")
        .join(elm)
}

#[test]
fn module_graphs_round_trip() {
    let mut checked = 0;
    for (elm, compat) in RELEASES {
        for case in fs::read_dir(artifacts(elm)).unwrap() {
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
    for (elm, compat) in RELEASES {
        let path = artifacts(elm).join("o.dat");
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
