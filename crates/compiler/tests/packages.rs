//! The elm/* packages under `tests/packages/` are the releases
//! `manifest.toml` names, file for file (`cargo xtask corpus fixtures`
//! copies them from the corpus, checked against the registry's hashes).

use std::fs;

use nomi_testkit::{files_below, packages, tree_digest};

#[test]
fn packages_are_the_pinned_releases() {
    let manifest = packages::manifest();
    assert_eq!(manifest.len(), 12);
    for package in manifest {
        let (name, version) = (&package.name, &package.version);
        let root = package.dir();
        let files = files_below(&root).unwrap();
        assert_eq!(files.len(), package.files, "{name}");
        assert_eq!(
            tree_digest(&root, &files).unwrap(),
            package.digest,
            "{name}"
        );
        let elm_json = fs::read_to_string(root.join("elm.json")).unwrap();
        assert!(
            elm_json.contains(&format!("\"name\": \"{name}\"")),
            "{name}"
        );
        assert!(
            elm_json.contains(&format!("\"version\": \"{version}\"")),
            "{name}"
        );
    }
}
