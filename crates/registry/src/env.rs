//! Where the solver finds releases: the index, the package store and the
//! website (`Solver.Env` and `getConstraints` in `Deps/Solver.hs`, the
//! same in every supported release).
//!
//! A release's `elm.json` comes from the store when it is downloaded, and
//! nomi's own packages' from the store's bundle alone.
//! Online, one that isn't comes from the website and is kept beside the
//! index, as upstream keeps it in `~/.elm`; offline, the search passes
//! over releases that aren't downloaded, since they couldn't be built:
//!
//! ```text
//! <metadata>/<author>/<project>/<version>.json
//! ```

use std::fs;
use std::io;
use std::path::PathBuf;

use nomi_base::Compat;
use nomi_project::{Name, Version};

use crate::files::{read_if_exists, write_atomically};
use crate::http::{DOMAIN, Http, HttpError};
use crate::index::{Connection, Index, Known};
use crate::solver::{Packages, Release};
use crate::store::Store;

/// The index and where releases' metadata comes from.
#[derive(Debug)]
pub struct Registry<'a, H> {
    /// The releases there are.
    pub index: Index,
    /// Whether `index` is the website's latest, and the website reachable.
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
    /// The website's doesn't decode (`SolverBadHttpData`).
    BadHttpData {
        name: Name,
        version: Version,
        url: String,
    },
    /// The website's couldn't be fetched (`SolverBadHttp`).
    BadHttp {
        name: Name,
        version: Version,
        url: String,
        error: HttpError,
    },
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
    /// `metadata`, or, `Online`, from the website. nomi's store, which
    /// serves nomi's own packages from its bundle, sees the index without
    /// the registry's releases of them.
    pub fn new(
        index: Index,
        connection: Connection,
        store: &'a Store,
        metadata: PathBuf,
        http: &'a H,
        compat: Compat,
    ) -> Registry<'a, H> {
        let index = if store.has_bundle() {
            index.without_bundled()
        } else {
            index
        };
        Registry {
            index,
            connection,
            store,
            metadata,
            http,
            compat,
        }
    }

    /// The store the releases are downloaded into.
    pub fn store(&self) -> &'a Store {
        self.store
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
        (self.index.versions(name)).or_else(|| self.store.bundle().versions(name))
    }

    fn offline(&self) -> bool {
        self.connection == Connection::Offline
    }

    fn release(&mut self, name: &Name, version: Version) -> Result<Option<Release>, SolverProblem> {
        let bad_cache = || SolverProblem::BadCacheData(name.clone(), version);
        if let Some(dir) = self.store.get(name, version)? {
            let bytes = fs::read(dir.join("elm.json"))?;
            return Release::decode(&bytes, self.compat)
                .map(Some)
                .ok_or_else(bad_cache);
        }
        if self.store.bundles(name) {
            return Ok(None);
        }
        let kept = self.kept(name, version);
        if let Some(bytes) = read_if_exists(&kept)? {
            let Some(release) = Release::decode(&bytes, self.compat) else {
                fs::remove_file(&kept)?;
                return Err(bad_cache());
            };
            return Ok(match self.connection {
                Connection::Online => Some(release),
                Connection::Offline => None,
            });
        }
        if self.connection == Connection::Offline {
            return Ok(None);
        }
        let url = format!("{DOMAIN}/packages/{name}/{version}/elm.json");
        let body = match self.http.get(&url) {
            Ok(body) => body,
            Err(error) => {
                return Err(SolverProblem::BadHttp {
                    name: name.clone(),
                    version,
                    url,
                    error,
                });
            }
        };
        let Some(release) = Release::decode(&body, self.compat) else {
            return Err(SolverProblem::BadHttpData {
                name: name.clone(),
                version,
                url,
            });
        };
        write_atomically(&kept, &body)?;
        Ok(Some(release))
    }
}
