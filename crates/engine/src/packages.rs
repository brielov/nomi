//! Downloading packages: each release the store lacks is fetched on the
//! pool, as upstream forks a thread per package (`Details.verifyDeps`).

use std::path::PathBuf;

use nomi_base::Compat;
use nomi_project::{Name, Version};
use nomi_registry::{Http, PackageProblem, Store};

use crate::Pool;

/// The sources of every release, in their order, each downloaded unless
/// the store already has it.
pub fn fetch(
    pool: &Pool,
    store: &Store,
    http: &(impl Http + Sync),
    compat: Compat,
    releases: &[(Name, Version)],
) -> Vec<Result<PathBuf, PackageProblem>> {
    pool.map(releases.iter().collect(), |(name, version)| {
        store.fetch(http, name, *version, compat)
    })
}
