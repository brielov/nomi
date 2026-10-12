//! The elm/* packages under `tests/packages/` are the releases
//! `manifest.toml` names, file for file (`cargo xtask corpus fixtures`
//! copies them from the corpus, checked against the registry's hashes).

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/packages")
}

/// Every file under `root`, relative and with `/`, sorted.
fn files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let relative = path.strip_prefix(root).unwrap();
                files.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    files.sort();
    files
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `sha256sum`'s listing of `files`, hashed, as the copier wrote it.
fn digest(root: &Path, files: &[String]) -> String {
    let mut listing = String::new();
    for file in files {
        let bytes = fs::read(root.join(file)).unwrap();
        listing += &format!("{}  ./{file}\n", hex(&Sha256::digest(&bytes)));
    }
    hex(&Sha256::digest(listing.as_bytes()))
}

#[test]
fn packages_are_the_pinned_releases() {
    let manifest = fs::read_to_string(dir().join("manifest.toml")).unwrap();
    let field = |entry: &str, key: &str| -> String {
        let line = entry
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{key} = ")))
            .unwrap_or_else(|| panic!("no {key} in {entry}"));
        line.trim_matches('"').to_owned()
    };
    let entries: Vec<&str> = manifest.split("[[package]]").skip(1).collect();
    assert_eq!(entries.len(), 12);
    for entry in entries {
        let (name, version) = (field(entry, "name"), field(entry, "version"));
        let root = dir().join(&name).join(&version);
        let files = files(&root);
        assert_eq!(files.len().to_string(), field(entry, "files"), "{name}");
        assert_eq!(digest(&root, &files), field(entry, "digest"), "{name}");
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
