//! The registry the recordings had, served from fixture releases, and
//! nomi run against it offline.
#![allow(dead_code)] // Each test target uses part.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use std::process::Command;

use nomi_base::Compat;
use nomi_project::Name;
use nomi_project::json::Cursor as JsonCursor;
use nomi_registry::{Http, HttpError, Index, Store};
use sha1::{Digest, Sha1};

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

/// The registry the recordings had: one release of each fixture package,
/// whose archive holds its `elm.json`, or all its files.
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
        let compat = Compat::LATEST;
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
                    let full = format!(
                        "{}/{}",
                        author.file_name().unwrap().to_string_lossy(),
                        project.file_name().unwrap().to_string_lossy()
                    );
                    let name =
                        Name::parse(full.as_bytes(), JsonCursor::start(compat), compat).unwrap();
                    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
                    let options = zip::write::SimpleFileOptions::default();
                    zip.add_directory("root/", options).unwrap();
                    let files = if sources {
                        files_below(&version)
                    } else {
                        vec!["elm.json".to_owned()]
                    };
                    for file in files {
                        zip.start_file(format!("root/{file}"), options).unwrap();
                        zip.write_all(&fs::read(version.join(&file)).unwrap())
                            .unwrap();
                    }
                    let number = version.file_name().unwrap().to_string_lossy().into_owned();
                    archives.insert((name, number), zip.finish().unwrap().into_inner());
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
            let hash: String = Sha1::digest(archive)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            let zip = url.replace("endpoint.json", "archive.zip");
            return Ok(format!(r#"{{"url":"{zip}","hash":"{hash}"}}"#).into_bytes());
        }
        Ok(archive.clone())
    }

    fn post(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        assert!(url.ends_with("/all-packages"), "an unexpected POST {url}");
        let mut all = BTreeMap::<String, Vec<String>>::new();
        for (name, version) in self.archives.keys() {
            all.entry(name.to_string())
                .or_default()
                .push(format!("\"{version}\""));
        }
        let entries: Vec<String> = all
            .iter()
            .map(|(name, versions)| format!("\"{name}\":[{}]", versions.join(",")))
            .collect();
        Ok(format!("{{{}}}", entries.join(",")).into_bytes())
    }
}

/// The files below `dir`, by `/`-separated relative paths.
fn files_below(dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().unwrap().is_dir() {
            files.extend(
                files_below(&entry.path())
                    .into_iter()
                    .map(|f| format!("{name}/{f}")),
            );
        } else {
            files.push(name);
        }
    }
    files.sort();
    files
}
