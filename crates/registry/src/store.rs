//! The package store: the source of every downloaded release, as
//! upstream's `downloadPackage` fetches it (`Elm/Details.hs`, the same in
//! every supported release). The website's `endpoint.json` names an
//! archive and its SHA-1; the archive is checked against it and its
//! sources are extracted.
//!
//! nomi never writes `~/.elm`. Its store keeps each archive's sources
//! once, under the archive's hash, and each release names the hash it
//! was published with:
//!
//! ```text
//! <store>/releases/<author>/<project>/<version>   the hash, in hex
//! <store>/sources/<hash>/                         elm.json, src/, ...
//! ```
//!
//! Both are written in one step, so a store interrupted or shared by two
//! processes holds whole releases only.

use std::fs;
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};

use nomi_base::Compat;
use nomi_project::json::{self, decode};
use nomi_project::{Name, Version};
use sha1::{Digest, Sha1};

use crate::http::{DOMAIN, Http, HttpError};

/// The most an archive may hold once extracted. The largest published
/// package is a few megabytes.
const EXTRACTED_LIMIT: u64 = 256 << 20;

/// The most entries an archive may have.
const ENTRY_LIMIT: usize = 1 << 16;

/// Downloaded releases.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

/// `Exit.PackageProblem`: why a release couldn't be downloaded.
#[derive(Debug)]
pub enum PackageProblem {
    /// `endpoint.json` couldn't be fetched (`PP_BadEndpointRequest`): its
    /// URL and why.
    BadEndpointRequest(String, HttpError),
    /// It isn't an endpoint (`PP_BadEndpointContent`): its URL.
    BadEndpointContent(String),
    /// The archive couldn't be fetched (`PP_BadArchiveRequest`).
    BadArchiveRequest(String, HttpError),
    /// The archive isn't a zip nomi will extract (`PP_BadArchiveContent`):
    /// its URL. Upstream extracts any zip; nomi also refuses entries that
    /// would land outside the release and archives too large to be one.
    BadArchiveContent(String),
    /// The archive isn't the one published (`PP_BadArchiveHash`): its URL,
    /// the expected hash and the actual one.
    BadArchiveHash(String, String, String),
    /// The store couldn't be read or written.
    Io(io::Error),
}

impl From<io::Error> for PackageProblem {
    fn from(e: io::Error) -> PackageProblem {
        PackageProblem::Io(e)
    }
}

impl Store {
    pub fn new(dir: PathBuf) -> Store {
        Store { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The sources of a release already downloaded.
    pub fn get(&self, name: &Name, version: Version) -> io::Result<Option<PathBuf>> {
        let hash = match fs::read(self.release(name, version)) {
            Ok(hash) => hash,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let sources = std::str::from_utf8(&hash)
            .ok()
            .and_then(|hash| hash.strip_suffix('\n'))
            .filter(|hash| is_hash(hash))
            .map(|hash| self.sources(hash));
        Ok(sources.filter(|dir| dir.is_dir()))
    }

    /// The releases of `name` already downloaded, oldest first.
    pub fn versions(&self, name: &Name) -> io::Result<Vec<Version>> {
        let dir = self
            .dir
            .join("releases")
            .join(name.author())
            .join(name.project());
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut versions = Vec::new();
        for entry in entries {
            let file = entry?.file_name();
            let parts: Option<Vec<u32>> = (file.to_str().unwrap_or("").split('.'))
                .map(|part| part.parse().ok())
                .collect();
            if let Some(&[major, minor, patch]) = parts.as_deref() {
                versions.push(Version::new(major, minor, patch));
            }
        }
        versions.sort();
        Ok(versions)
    }

    /// The sources of a release, downloaded unless they already are.
    pub fn fetch(
        &self,
        http: &impl Http,
        name: &Name,
        version: Version,
        compat: Compat,
    ) -> Result<PathBuf, PackageProblem> {
        if let Some(dir) = self.get(name, version)? {
            return Ok(dir);
        }
        let (url, expected) = endpoint(http, name, version, compat)?;
        let archive = http
            .get(&url)
            .map_err(|e| PackageProblem::BadArchiveRequest(url.clone(), e))?;
        // Upstream unzips as it downloads, so a broken zip is reported as
        // one before its hash is checked.
        let Ok(mut zip) = zip::ZipArchive::new(Cursor::new(&archive[..])) else {
            return Err(PackageProblem::BadArchiveContent(url));
        };
        let actual = hex(&Sha1::digest(&archive));
        if expected != actual {
            return Err(PackageProblem::BadArchiveHash(url, expected, actual));
        }
        let dir = self.sources(&actual);
        if !dir.is_dir() {
            self.extract(&mut zip, &dir, &url)?;
        }
        let release = self.release(name, version);
        let parent = release.parent().unwrap_or(&self.dir);
        fs::create_dir_all(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        writeln!(temp, "{actual}")?;
        temp.persist(&release).map_err(|e| e.error)?;
        Ok(dir)
    }

    fn release(&self, name: &Name, version: Version) -> PathBuf {
        self.dir
            .join("releases")
            .join(name.author())
            .join(name.project())
            .join(version.to_string())
    }

    fn sources(&self, hash: &str) -> PathBuf {
        self.dir.join("sources").join(hash)
    }

    /// Extracts beside `dir`, then moves the whole into place. When
    /// another process got there first, its copy serves.
    fn extract(
        &self,
        zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
        dir: &Path,
        url: &str,
    ) -> Result<(), PackageProblem> {
        let parent = dir.parent().unwrap_or(&self.dir);
        fs::create_dir_all(parent)?;
        let temp = tempfile::Builder::new()
            .prefix(".extracting")
            .tempdir_in(parent)?;
        match write_package(zip, temp.path()) {
            Ok(()) => {}
            Err(Extract::Refused) => return Err(PackageProblem::BadArchiveContent(url.to_owned())),
            Err(Extract::Io(e)) => return Err(e.into()),
        }
        match fs::rename(temp.path(), dir) {
            Ok(()) => Ok(()),
            Err(_) if dir.is_dir() => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// `endpointDecoder` over `endpoint.json`: the archive's URL and hash.
fn endpoint(
    http: &impl Http,
    name: &Name,
    version: Version,
    compat: Compat,
) -> Result<(String, String), PackageProblem> {
    let url = format!("{DOMAIN}/packages/{name}/{version}/endpoint.json");
    let body = http
        .get(&url)
        .map_err(|e| PackageProblem::BadEndpointRequest(url.clone(), e))?;
    let decoded = json::parse(&body, compat).ok().and_then(|value| {
        // 0.19.1 reads the strings with their escapes decoded; the website
        // writes none.
        let string = |key: &[u8]| -> Option<String> {
            let bytes = decode::field::<_, ()>(&value, key, decode::string).ok()?;
            String::from_utf8(bytes.to_vec()).ok()
        };
        Some((string(b"url")?, string(b"hash")?))
    });
    decoded.ok_or(PackageProblem::BadEndpointContent(url))
}

enum Extract {
    /// An entry nomi won't write, or more than a package holds.
    Refused,
    Io(io::Error),
}

impl From<io::Error> for Extract {
    fn from(e: io::Error) -> Extract {
        Extract::Io(e)
    }
}

/// `writePackage`: every entry but the first, without the first's name
/// (GitHub's top directory) in front, as 0.19.1 and 0.19.2 keep them:
/// `elm.json`, `LICENSE`, `README.md` and all of `src/`. 0.19.3 keeps only
/// the `.elm` files and kernel `.js` files of `src/`, which nothing else
/// there could shadow, so one copy serves every release.
fn write_package(zip: &mut zip::ZipArchive<Cursor<&[u8]>>, dir: &Path) -> Result<(), Extract> {
    if zip.len() > ENTRY_LIMIT {
        return Err(Extract::Refused);
    }
    if zip.is_empty() {
        return Ok(());
    }
    let root = zip
        .by_index_raw(0)
        .map_err(|_| Extract::Refused)?
        .name_raw()
        .len();
    let mut budget = EXTRACTED_LIMIT;
    for i in 1..zip.len() {
        let mut entry = zip.by_index(i).map_err(|_| Extract::Refused)?;
        let path = entry.name_raw().get(root..).unwrap_or_default();
        let kept = path.starts_with(b"src/")
            || path == b"LICENSE"
            || path == b"README.md"
            || path == b"elm.json";
        if !kept {
            continue;
        }
        let (relative, is_dir) = match path.strip_suffix(b"/") {
            Some(dir) => (dir, true),
            None => (path, false),
        };
        let target = dir.join(safe(relative).ok_or(Extract::Refused)?);
        if is_dir {
            fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        // A symlink's entry holds its target, which upstream writes as
        // the file's contents; nomi never makes links either.
        let mut bytes = Vec::new();
        (&mut entry)
            .take(budget + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Extract::Refused)?;
        budget = budget
            .checked_sub(bytes.len() as u64)
            .ok_or(Extract::Refused)?;
        fs::write(&target, bytes)?;
    }
    Ok(())
}

/// An entry's path as relative components that stay inside the release
/// on every platform, or `None`.
fn safe(path: &[u8]) -> Option<PathBuf> {
    let path = std::str::from_utf8(path).ok()?;
    let mut safe = PathBuf::new();
    for part in path.split('/') {
        let bad =
            |c: char| c.is_control() || matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|');
        if part.is_empty() || part == "." || part == ".." || part.contains(bad) {
            return None;
        }
        safe.push(part);
    }
    Some(safe)
}

/// `SHA.showDigest`: lowercase hex.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn is_hash(hash: &str) -> bool {
    hash.len() == 40 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
