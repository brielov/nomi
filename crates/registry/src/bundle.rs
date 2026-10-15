//! The packages bundled with nomi, `nomi/*`: `packages/`, a directory a
//! package, embedded in the binary and extracted into the cache once, under
//! a hash of their files:
//!
//! ```text
//! <bundled>/<hash>/<project>/   elm.json, src/, ...
//! ```
//!
//! None comes from the registry: the index drops every release of
//! `nomi/*`, and the store serves them from the bundle alone, which is
//! what lets [`kernel::is_author`] trust their kernel code.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use nomi_base::{Compat, kernel};
use nomi_project::{Name, Outline, Version};

use crate::files::read_if_exists;
use crate::index::Known;
use crate::store::sha1;

/// Each path under `packages/`, with its bytes.
macro_rules! files {
    ($($path:literal),* $(,)?) => {
        [$(($path, include_bytes!(concat!("../../../packages/", $path)).as_slice())),*]
    };
}

/// The bundled packages' files, by path under `packages/`.
const FILES: [(&str, &[u8]); 5] = files![
    "core/elm.json",
    "core/src/Elm/Kernel/Nomi.js",
    "core/src/Nomi.elm",
    "core/src/Nomi/Fiber.elm",
    "core/src/Nomi/Task.elm",
];

/// The extracted bundle's directory name, by a hash of its files.
static NAME: LazyLock<String> = LazyLock::new(|| {
    let mut all = Vec::new();
    for (path, bytes) in FILES {
        all.extend_from_slice(path.as_bytes());
        all.push(0);
        all.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        all.extend_from_slice(bytes);
    }
    sha1(&all)[..16].to_owned()
});

/// A variable naming a directory laid out as `packages/` is, whose
/// packages replace the bundled ones, as tests bundle fixtures.
const BUNDLE: &str = "NOMI_BUNDLE";

/// Whether `name` is one of nomi's own, which only nomi bundles.
#[inline]
pub fn is_bundled(name: &Name) -> bool {
    name.author() == kernel::BUNDLED_AUTHOR
}

/// The bundled packages, each at its one version.
#[derive(Clone, Debug, Default)]
pub struct Bundle {
    packages: BTreeMap<Name, (Known, PathBuf)>,
}

impl Bundle {
    /// The packages nomi bundles, extracted under `dir` unless they already
    /// are, or those in the directory `NOMI_BUNDLE` names.
    pub fn locate(dir: &Path) -> io::Result<Bundle> {
        match std::env::var_os(BUNDLE) {
            Some(fixture) => Bundle::read(Path::new(&fixture)),
            None => Bundle::read(&extract(dir)?),
        }
    }

    /// The packages in `dir`, one in each directory whose `elm.json`
    /// names one of nomi's.
    pub fn read(dir: &Path) -> io::Result<Bundle> {
        let mut packages = BTreeMap::new();
        for entry in fs::read_dir(dir)? {
            let dir = entry?.path();
            let Some(bytes) = read_if_exists(&dir.join("elm.json"))? else {
                continue;
            };
            if let Ok(Outline::Pkg(pkg)) = Outline::decode(&bytes, Compat::LATEST)
                && is_bundled(&pkg.name)
            {
                let known = Known {
                    newest: pkg.version,
                    previous: Vec::new(),
                };
                packages.insert(pkg.name, (known, dir));
            }
        }
        Ok(Bundle { packages })
    }

    /// The one release of `name` bundled, if it is.
    pub fn versions(&self, name: &Name) -> Option<&Known> {
        self.packages.get(name).map(|(known, _)| known)
    }

    /// The sources of `name` at `version`, if that is the release bundled.
    pub fn get(&self, name: &Name, version: Version) -> Option<&Path> {
        (self.packages.get(name))
            .filter(|(known, _)| known.newest == version)
            .map(|(_, dir)| dir.as_path())
    }

    /// Each bundled package, in order of names, with its release and its
    /// sources.
    pub fn packages(&self) -> impl Iterator<Item = (&Name, &Known, &Path)> {
        (self.packages.iter()).map(|(name, (known, dir))| (name, known, dir.as_path()))
    }
}

/// The embedded files under `dir`, in a directory named by their hash,
/// written aside and moved in whole unless they already are there, so a
/// run never reads them half written.
fn extract(dir: &Path) -> io::Result<PathBuf> {
    let target = dir.join(&*NAME);
    if target.is_dir() {
        return Ok(target);
    }
    fs::create_dir_all(dir)?;
    let temp = tempfile::Builder::new()
        .prefix(".extracting")
        .tempdir_in(dir)?;
    for (path, bytes) in FILES {
        let path = temp.path().join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, bytes)?;
    }
    // Another run may have moved its copy in first.
    match fs::rename(temp.path(), &target) {
        Err(e) if !target.is_dir() => Err(e),
        _ => Ok(target),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_is_extracted_once_and_reads_back_whole() {
        let cache = tempfile::tempdir().unwrap();
        let dir = extract(cache.path()).unwrap();
        assert_eq!(extract(cache.path()).unwrap(), dir);
        assert_eq!(fs::read_dir(cache.path()).unwrap().count(), 1);
        for (path, bytes) in FILES {
            assert_eq!(fs::read(dir.join(path)).unwrap(), bytes, "{path}");
        }
        // Each top directory holds a package of nomi's.
        let projects: std::collections::BTreeSet<&str> = (FILES.iter())
            .filter_map(|(path, _)| path.split_once('/'))
            .map(|(project, _)| project)
            .collect();
        let bundle = Bundle::read(&dir).unwrap();
        let read: Vec<&str> = bundle.packages().map(|(name, ..)| name.project()).collect();
        assert_eq!(read, Vec::from_iter(projects));
    }

    #[test]
    fn the_bundle_holds_every_file_under_packages() {
        fn walk(dir: &Path, base: &Path, files: &mut Vec<String>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, base, files);
                } else {
                    let relative = path.strip_prefix(base).unwrap();
                    files.push(relative.to_string_lossy().into_owned());
                }
            }
        }
        let packages = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages");
        let mut files = Vec::new();
        walk(&packages, &packages, &mut files);
        files.sort();
        let bundled: Vec<&str> = FILES.iter().map(|(path, _)| *path).collect();
        assert_eq!(bundled, files);
    }
}
