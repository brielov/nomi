//! The package index: every published release, as upstream keeps it
//! (`Deps/Registry.hs`, the same in every supported release). The first
//! request takes `/all-packages`, a map from names to versions; each later
//! one takes `/all-packages/since/N`, the releases after the first N,
//! newest first.
//!
//! nomi never writes upstream's `registry.dat`. Its own index file keeps
//! the website's responses as they came, so each Compat decodes them as
//! its elm would, and the index works offline once fetched.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use nomi_base::Compat;
use nomi_project::json::{self, decode};
use nomi_project::{Name, Version};

use crate::http::{DOMAIN, Http, HttpError};

/// The first line of an index file, naming its format.
const MAGIC: &[u8] = b"nomi package index 1\n";

/// The releases of every package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Index {
    /// How many releases the website has listed, which the next update
    /// asks for releases after.
    count: usize,
    packages: BTreeMap<Name, Known>,
    /// The responses this index was built from, oldest first.
    responses: Vec<Vec<u8>>,
}

/// `KnownVersions`: a package's releases, newest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Known {
    pub newest: Version,
    pub previous: Vec<Version>,
}

impl Known {
    /// Every release, newest first.
    pub fn all(&self) -> impl Iterator<Item = Version> + '_ {
        std::iter::once(self.newest).chain(self.previous.iter().copied())
    }
}

/// Whether the index is the website's latest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    Online,
    /// The update failed, so the index is the one saved last; upstream
    /// then builds only with packages it already has.
    Offline,
}

/// `Exit.RegistryProblem`: why there is no index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryProblem {
    /// The website couldn't be reached (`RP_Http`).
    Http(String, HttpError),
    /// It answered with what isn't an index (`RP_Data`): the URL and body.
    Data(String, Vec<u8>),
}

impl Index {
    /// `latest`: the saved index brought up to date and saved again, or,
    /// with none saved, the whole index fetched and saved. When the update
    /// fails, the saved index serves as it is, offline.
    pub fn latest(
        http: &impl Http,
        path: &Path,
        compat: Compat,
    ) -> Result<(Index, Connection), LatestError> {
        match Index::read(path, compat).map_err(LatestError::Io)? {
            None => {
                let index = Index::fetch(http, compat).map_err(LatestError::Registry)?;
                index.write(path).map_err(LatestError::Io)?;
                Ok((index, Connection::Online))
            }
            Some(saved) => match saved.update(http, compat) {
                Ok(None) => Ok((saved, Connection::Online)),
                Ok(Some(index)) => {
                    index.write(path).map_err(LatestError::Io)?;
                    Ok((index, Connection::Online))
                }
                Err(_) => Ok((saved, Connection::Offline)),
            },
        }
    }

    /// `fetch`: the whole index.
    pub fn fetch(http: &impl Http, compat: Compat) -> Result<Index, RegistryProblem> {
        let url = format!("{DOMAIN}/all-packages");
        let body = http
            .post(&url)
            .map_err(|e| RegistryProblem::Http(url.clone(), e))?;
        match all_packages(&body, compat) {
            Some(packages) => Ok(Index::new(packages, body)),
            None => Err(RegistryProblem::Data(url, body)),
        }
    }

    /// `update`: the index with the releases published since it was
    /// fetched, or `None` when there are none.
    pub fn update(
        &self,
        http: &impl Http,
        compat: Compat,
    ) -> Result<Option<Index>, RegistryProblem> {
        let url = format!("{DOMAIN}/all-packages/since/{}", self.count);
        let body = http
            .post(&url)
            .map_err(|e| RegistryProblem::Http(url.clone(), e))?;
        let Some(news) = since(&body, compat) else {
            return Err(RegistryProblem::Data(url, body));
        };
        if news.is_empty() {
            return Ok(None);
        }
        let mut index = self.clone();
        index.add(news);
        index.responses.push(body);
        Ok(Some(index))
    }

    /// The index saved at `path`, as `compat` reads it, or `None` when
    /// there is none or it can't be read, as upstream treats a corrupt
    /// `registry.dat`.
    pub fn read(path: &Path, compat: Compat) -> io::Result<Option<Index>> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        Ok(Index::decode(&bytes, compat))
    }

    /// Saves the index at `path` in one step, creating its directory.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        let dir = path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(dir)?;
        let mut temp = tempfile::NamedTempFile::new_in(dir)?;
        temp.write_all(&self.encode())?;
        temp.persist(path).map_err(|e| e.error)?;
        Ok(())
    }

    /// `getVersions`.
    pub fn versions(&self, name: &Name) -> Option<&Known> {
        self.packages.get(name)
    }

    /// `getVersions'`: a package's releases, or the names nearest to its
    /// when there is no such package.
    pub fn versions_or_nearby(&self, name: &Name) -> Result<&Known, Vec<&Name>> {
        self.packages
            .get(name)
            .ok_or_else(|| name.nearby(self.packages.keys()))
    }

    pub fn packages(&self) -> impl Iterator<Item = (&Name, &Known)> {
        self.packages.iter()
    }

    /// How many releases the website has listed.
    pub fn count(&self) -> usize {
        self.count
    }

    fn new(packages: BTreeMap<Name, Known>, body: Vec<u8>) -> Index {
        // `addEntry`.
        let count = packages.values().map(|k| 1 + k.previous.len()).sum();
        Index {
            count,
            packages,
            responses: vec![body],
        }
    }

    /// `addNew` over `news`, newest first: `foldr` adds the last first,
    /// so the first becomes the newest.
    fn add(&mut self, news: Vec<(Name, Version)>) {
        self.count += news.len();
        for (name, version) in news.into_iter().rev() {
            match self.packages.get_mut(&name) {
                Some(known) => {
                    let previous = std::mem::replace(&mut known.newest, version);
                    known.previous.insert(0, previous);
                }
                None => {
                    let known = Known {
                        newest: version,
                        previous: Vec::new(),
                    };
                    self.packages.insert(name, known);
                }
            }
        }
    }

    /// The magic line, then each response as its length in decimal, a
    /// newline, its bytes and a newline.
    fn encode(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        for body in &self.responses {
            out.extend_from_slice(format!("{}\n", body.len()).as_bytes());
            out.extend_from_slice(body);
            out.push(b'\n');
        }
        out
    }

    fn decode(bytes: &[u8], compat: Compat) -> Option<Index> {
        let mut rest = bytes.strip_prefix(MAGIC)?;
        let mut responses = Vec::new();
        while !rest.is_empty() {
            let line = rest.iter().position(|&b| b == b'\n')?;
            let len: usize = std::str::from_utf8(&rest[..line]).ok()?.parse().ok()?;
            let end = (line + 1).checked_add(len)?;
            let body = rest.get(line + 1..end)?;
            rest = rest[end..].strip_prefix(b"\n")?;
            responses.push(body.to_vec());
        }
        let mut responses = responses.into_iter();
        let first = responses.next()?;
        let mut index = Index::new(all_packages(&first, compat)?, first);
        for body in responses {
            let news = since(&body, compat)?;
            index.add(news);
            index.responses.push(body);
        }
        Some(index)
    }
}

/// Why [`Index::latest`] has no index.
#[derive(Debug)]
pub enum LatestError {
    Registry(RegistryProblem),
    /// The saved index couldn't be read or written.
    Io(io::Error),
}

/// `allPkgsDecoder`: each package's versions sorted newest first, none
/// without any.
fn all_packages(body: &[u8], compat: Compat) -> Option<BTreeMap<Name, Known>> {
    let value = json::parse(body, compat).ok()?;
    let versions = |v: &json::Value<'_>| {
        let mut versions = decode::list(v, |e| {
            decode::custom_string(e, |bytes, start| {
                Version::parse(bytes, start, compat).map_err(|_| ())
            })
        })?;
        versions.sort_by(|a, b| b.cmp(a));
        let mut versions = versions.into_iter();
        match versions.next() {
            Some(newest) => Ok(Known {
                newest,
                previous: versions.collect(),
            }),
            None => decode::failure(v, ()),
        }
    };
    let key = |bytes: &[u8], start| Name::parse(bytes, start, compat).map_err(|_| ());
    decode::dict(&value, key, versions).ok()
}

/// `D.list newPkgDecoder`.
fn since(body: &[u8], compat: Compat) -> Option<Vec<(Name, Version)>> {
    let value = json::parse(body, compat).ok()?;
    decode::list(&value, |e| {
        decode::custom_string(e, |bytes, start| {
            Name::parse_release(bytes, start, compat).map_err(|_| ())
        })
    })
    .ok()
}
