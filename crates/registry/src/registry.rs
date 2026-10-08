//! Where the solver finds releases: the index, the package store and the
//! website (`Solver.Env` and `getConstraints` in `Deps/Solver.hs`, the
//! same in every supported release).
//!
//! A release's `elm.json` comes from the store when it is downloaded.
//! Online, one that isn't comes from the website and is kept beside the
//! index, as upstream keeps it in `~/.elm`; offline, the search passes
//! over releases that aren't downloaded, since they couldn't be built:
//!
//! ```text
//! <metadata>/<author>/<project>/<version>.json
//! ```

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use nomi_base::Compat;
use nomi_project::{Name, Version};

use crate::http::{DOMAIN, Http, HttpError};
use crate::index::{Connection, Index, Known};
use crate::solver::{Packages, Release};
use crate::store::Store;

/// The index and where releases' metadata comes from.
#[derive(Debug)]
pub struct Registry<'a, H> {
    pub index: Index,
    pub connection: Connection,
    store: &'a Store,
    metadata: PathBuf,
    http: &'a H,
    compat: Compat,
}

/// `Exit.Solver`: why a release's `elm.json` couldn't be read, which ends
/// the search.
#[derive(Debug)]
pub enum SolverProblem {
    /// The copy nomi kept doesn't decode (`SolverBadCacheData`). One kept
    /// beside the index is deleted, so the next run fetches it again.
    BadCacheData(Name, Version),
    /// The website's doesn't decode (`SolverBadHttpData`): its URL.
    BadHttpData(Name, Version, String),
    /// The website's couldn't be fetched (`SolverBadHttp`): its URL and why.
    BadHttp(Name, Version, String, HttpError),
    /// The store or the metadata couldn't be read or written.
    Io(io::Error),
}

impl From<io::Error> for SolverProblem {
    fn from(e: io::Error) -> SolverProblem {
        SolverProblem::Io(e)
    }
}

impl<'a, H: Http> Registry<'a, H> {
    /// The releases in `index`, with their metadata from `store`, from
    /// `metadata`, or, `Online`, from the website.
    pub fn new(
        index: Index,
        connection: Connection,
        store: &'a Store,
        metadata: PathBuf,
        http: &'a H,
        compat: Compat,
    ) -> Registry<'a, H> {
        Registry {
            index,
            connection,
            store,
            metadata,
            http,
            compat,
        }
    }

    fn kept(&self, name: &Name, version: Version) -> PathBuf {
        self.metadata
            .join(name.author())
            .join(name.project())
            .join(format!("{version}.json"))
    }
}

impl<H: Http> Packages for Registry<'_, H> {
    type Error = SolverProblem;

    fn versions(&self, name: &Name) -> Option<&Known> {
        self.index.versions(name)
    }

    fn release(&mut self, name: &Name, version: Version) -> Result<Option<Release>, SolverProblem> {
        let bad_cache = || SolverProblem::BadCacheData(name.clone(), version);
        if let Some(dir) = self.store.get(name, version)? {
            let bytes = fs::read(dir.join("elm.json"))?;
            return Release::decode(&bytes, self.compat)
                .map(Some)
                .ok_or_else(bad_cache);
        }
        let kept = self.kept(name, version);
        match fs::read(&kept) {
            Ok(bytes) => {
                let Some(release) = Release::decode(&bytes, self.compat) else {
                    fs::remove_file(&kept)?;
                    return Err(bad_cache());
                };
                return Ok(match self.connection {
                    Connection::Online => Some(release),
                    Connection::Offline => None,
                });
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        if self.connection == Connection::Offline {
            return Ok(None);
        }
        let url = format!("{DOMAIN}/packages/{name}/{version}/elm.json");
        let body = self
            .http
            .get(&url)
            .map_err(|e| SolverProblem::BadHttp(name.clone(), version, url.clone(), e))?;
        let Some(release) = Release::decode(&body, self.compat) else {
            return Err(SolverProblem::BadHttpData(name.clone(), version, url));
        };
        keep(&kept, &body)?;
        Ok(Some(release))
    }
}

/// Writes `bytes` at `path` in one step, creating its directory.
fn keep(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    temp.write_all(bytes)?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
