//! The fixture packages: elm/* releases under
//! `crates/compiler/tests/packages/`, file for file, which
//! `manifest.toml` lists with the registry's hash of each and a digest of
//! its files (`cargo xtask corpus fixtures` writes both).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Where the fixture packages are, each at `<author>/<project>/<version>/`.
pub fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/packages")
}

/// A bundle of nomi's own packages for tests, laid out as `packages/` is:
/// nomi/fixture 1.0.0, whose `Nomi.Fixture.shout` is its kernel's.
pub fn bundle() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("bundle")
}

/// The `elm.json` of an application of `src` that depends on elm/core
/// and elm/json alone.
pub const CORE_AND_JSON_APP: &str = r#"{
    "type": "application",
    "source-directories": ["src"],
    "elm-version": "0.19.1",
    "dependencies": { "direct": { "elm/core": "1.0.5", "elm/json": "1.1.4" }, "indirect": {} },
    "test-dependencies": { "direct": {}, "indirect": {} }
}
"#;

/// One release, as `manifest.toml` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    /// `<author>/<project>`.
    pub name: String,
    pub version: String,
    /// The registry's SHA-1 of the release's archive.
    pub sha1: String,
    /// How many files the release has.
    pub files: usize,
    /// The [`tree_digest`](crate::tree_digest) of its files.
    pub digest: String,
}

impl Package {
    /// The release's files.
    pub fn dir(&self) -> PathBuf {
        dir().join(&self.name).join(&self.version)
    }

    /// Where [`store`] puts the release's files in the store at `store`.
    pub fn sources(&self, store: &Path) -> PathBuf {
        store.join("sources").join(&self.sha1)
    }
}

/// The releases `manifest.toml` lists, in its order.
pub fn manifest() -> Vec<Package> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Manifest {
        package: Vec<Package>,
    }
    let path = dir().join("manifest.toml");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let manifest: Manifest =
        toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    manifest.package
}

/// What the package website's `all-packages` lists of the fixtures: each
/// package by name, and its one release.
pub fn all_packages() -> Vec<u8> {
    let index: serde_json::Map<String, serde_json::Value> = (manifest().into_iter())
        .map(|package| (package.name, serde_json::json!([package.version])))
        .collect();
    serde_json::to_vec(&index).expect("a JSON object encodes")
}

/// The release of the package `name`, `<author>/<project>`, among the
/// fixtures, which hold one of each.
pub fn release(name: &str) -> Package {
    (manifest().into_iter())
        .find(|package| package.name == name)
        .unwrap_or_else(|| panic!("{name} isn't a fixture package"))
}

/// Lays the fixture packages out in `store` as nomi's package store keeps
/// downloads: `releases/<name>/<version>` holding the archive's hash, and
/// the release's files at [`Package::sources`]. Gives back the packages.
pub fn store(store: &Path) -> io::Result<Vec<Package>> {
    let packages = manifest();
    for package in &packages {
        let releases = store.join("releases").join(&package.name);
        fs::create_dir_all(&releases)?;
        fs::write(
            releases.join(&package.version),
            format!("{}\n", package.sha1),
        )?;
        crate::copy_tree(&package.dir(), &package.sources(store), |_| true)?;
    }
    Ok(packages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_store_holds_every_release() {
        let temp = tempfile::tempdir().unwrap();
        let packages = store(temp.path()).unwrap();
        let core = (packages.iter())
            .find(|p| p.name == "elm/core")
            .expect("elm/core is a fixture package");
        let release = temp.path().join("releases/elm/core").join(&core.version);
        assert_eq!(
            fs::read_to_string(release).unwrap(),
            format!("{}\n", core.sha1)
        );
        let sources = core.sources(temp.path());
        assert_eq!(crate::files_below(&sources).unwrap().len(), core.files);
        assert!(sources.join("elm.json").is_file());
    }
}
