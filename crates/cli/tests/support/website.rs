//! The registry the recordings had, served from fixture releases, and
//! nomi run against it offline, under its own name or upstream's.
//! Shared by the test targets that run nomi, each taking it as a public
//! module, as each uses part of it.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use nomi_base::Compat;
use nomi_project::Name;
use nomi_registry::{Http, HttpError, Index, Store};
use nomi_testkit::{files_below, packages};
use tempfile::TempDir;

/// nomi copied into a new directory as `name`, which makes it the
/// upstream tool of that name; the directory goes when the first is
/// dropped.
pub fn personality(name: &str) -> (TempDir, PathBuf) {
    let bin = tempfile::tempdir().unwrap();
    let path = bin
        .path()
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_nomi"), &path).unwrap();
    (bin, path)
}

/// `program` with the cache at `cache`, where every request fails, as
/// the recordings' did.
pub fn offline(program: &Path, cache: &Path) -> Command {
    let mut command = Command::new(program);
    command.env("NOMI_CACHE", cache);
    for var in [
        "NO_PROXY",
        "no_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
    ] {
        command.env_remove(var);
    }
    command.env("ALL_PROXY", "http://127.0.0.1:9");
    command
}

/// A cache that knows and holds the packages vendored for the compiler's
/// tests.
pub fn fixture_cache() -> TempDir {
    let cache = tempfile::tempdir().unwrap();
    Website::with_sources(&packages::dir()).fill(cache.path());
    cache
}

/// The `files` of `dir`, zipped under the directory `top`, as GitHub
/// zips a tag.
pub fn zipped(top: &str, dir: &Path, files: &[impl AsRef<str>]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    zip.add_directory(format!("{top}/"), options).unwrap();
    for file in files {
        let file = file.as_ref();
        zip.start_file(format!("{top}/{file}"), options).unwrap();
        zip.write_all(&fs::read(dir.join(file)).unwrap()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// The registry the recordings had: one release of each fixture package,
/// whose archive holds its `elm.json`, or all its files.
#[derive(Debug)]
pub struct Website {
    archives: BTreeMap<(Name, String), Vec<u8>>,
}

impl Website {
    /// The releases under `packages`, at `<author>/<project>/<version>/`,
    /// each archive holding only its `elm.json`.
    pub fn new(packages: &Path) -> Website {
        Website::serving(packages, false)
    }

    /// The releases under `packages`, each archive holding every file.
    pub fn with_sources(packages: &Path) -> Website {
        Website::serving(packages, true)
    }

    fn serving(packages: &Path, sources: bool) -> Website {
        let mut archives = BTreeMap::new();
        for author in fs::read_dir(packages).unwrap() {
            let author = author.unwrap().path();
            if !author.is_dir() {
                continue;
            }
            for project in fs::read_dir(&author).unwrap() {
                let project = project.unwrap().path();
                for version in fs::read_dir(&project).unwrap() {
                    let version = version.unwrap().path();
                    let name: Name = format!(
                        "{}/{}",
                        author.file_name().unwrap().to_string_lossy(),
                        project.file_name().unwrap().to_string_lossy()
                    )
                    .parse()
                    .unwrap();
                    let files = if sources {
                        files_below(&version).unwrap()
                    } else {
                        vec!["elm.json".to_owned()]
                    };
                    let number = version.file_name().unwrap().to_string_lossy().into_owned();
                    archives.insert((name, number), zipped("root", &version, &files));
                }
            }
        }
        Website { archives }
    }

    /// Saves the index in the cache at `dir` and downloads every release
    /// into its store.
    pub fn fill(&self, dir: &Path) {
        let compat = Compat::LATEST;
        let index = Index::fetch(self, compat).unwrap();
        index.write(&dir.join("registry").join("index")).unwrap();
        let store = Store::new(dir.join("packages"));
        for name in self.archives.keys().map(|(name, _)| name) {
            let known = index.versions(name).unwrap();
            for version in known.all() {
                store.fetch(self, name, version, compat).unwrap();
            }
        }
    }

    fn archive(&self, url: &str) -> Option<&Vec<u8>> {
        self.archives.iter().find_map(|((name, version), archive)| {
            url.contains(&format!("/{name}/{version}/"))
                .then_some(archive)
        })
    }
}

impl Http for Website {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        let archive = self.archive(url).ok_or(HttpError::Status(404))?;
        if url.ends_with("endpoint.json") {
            let hash = nomi_registry::store::sha1(archive);
            let zip = url.replace("endpoint.json", "archive.zip");
            return Ok(format!(r#"{{"url":"{zip}","hash":"{hash}"}}"#).into_bytes());
        }
        Ok(archive.clone())
    }

    fn post(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        assert!(url.ends_with("/all-packages"), "an unexpected POST {url}");
        let mut all = BTreeMap::<String, Vec<&str>>::new();
        for (name, version) in self.archives.keys() {
            all.entry(name.to_string()).or_default().push(version);
        }
        Ok(serde_json::to_vec(&all).unwrap())
    }
}
