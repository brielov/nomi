//! The dependencies' graph of elm/core and elm/json, which the optimizer
//! oracle's last case builds with, encodes to each elm's `o.dat` of it
//! (`cargo xtask oracle optimized`); and package builds come back from
//! the artifact cache whole.

use std::fs;
use std::path::Path;

use nomi_base::Compat;
use nomi_compiler::optimized::binary;
use nomi_engine::artifacts::Artifacts;
use nomi_engine::objects::{self, dependency_graph};
use nomi_engine::paths::Cache;
use nomi_project::{Name, json::Cursor};

/// elm/core and elm/json as vendored, copied under `to` when given.
fn core_and_json(compat: Compat, to: Option<&Path>) -> Vec<(Name, std::path::PathBuf)> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let vendored = manifest.join("../compiler/tests/packages/elm");
    ["core", "json"]
        .into_iter()
        .map(|project| {
            let mut dir = fs::read_dir(vendored.join(project))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            if let Some(to) = to {
                let copy = to.join(project);
                copy_dir(&dir, &copy);
                dir = copy;
            }
            let full = format!("elm/{project}");
            let name = Name::parse(full.as_bytes(), Cursor::start(compat), compat).unwrap();
            (name, dir)
        })
        .collect()
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap();
        }
    }
}

#[test]
fn a_second_build_reads_packages_from_the_cache() {
    let temp = tempfile::tempdir().unwrap();
    let artifacts = Artifacts::new(&Cache::new(temp.path().join("cache")));
    for compat in [Compat::V0_19_1, Compat::V0_19_3] {
        let sources = temp.path().join(compat.to_string());
        let packages = core_and_json(compat, Some(&sources));
        let memo = &mut objects::Memo::new();
        let first = objects::build(compat, &packages, memo, Some(&artifacts)).unwrap();
        // Without their modules, the packages can only come from the cache.
        for (_, dir) in &packages {
            fs::remove_dir_all(dir.join("src")).unwrap();
        }
        let memo = &mut objects::Memo::new();
        let second = objects::build(compat, &packages, memo, Some(&artifacts)).unwrap();
        assert_eq!(first.exposed(), second.exposed());
        assert_eq!(first.graph(), second.graph());
        let types = |d: &objects::Dependencies| {
            let mut types: Vec<_> = d.types().into_iter().map(|(h, t)| (h.module, t)).collect();
            types.sort_by(|a, b| a.0.cmp(&b.0));
            format!("{types:?}")
        };
        assert_eq!(types(&first), types(&second));
        // Built afresh, they now have no modules to build.
        let uncached = objects::build(compat, &packages, &mut objects::Memo::new(), None).unwrap();
        assert!(uncached.graph().nodes.len() < first.graph().nodes.len());
    }
}

#[test]
fn core_and_json_merge_as_elm_merges_them() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    for compat in [Compat::V0_19_1, Compat::V0_19_2, Compat::V0_19_3] {
        let packages = core_and_json(compat, None);
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
