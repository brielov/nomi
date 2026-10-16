//! The package store, with the packages bundled with nomi, and
//! downloading: each release the store lacks is fetched on the pool, as
//! upstream forks a thread per package (`Details.verifyDeps`).

use std::io;
use std::path::PathBuf;

use nomi_base::{Compat, Report};
use nomi_project::{Name, Version};
use nomi_registry::{Bundle, Http, PackageProblem, Store};

use crate::Pool;
use crate::console::Personality;
use crate::paths::Cache;
use crate::report::one_line;

/// The package store in `cache`, with the packages bundled with nomi.
pub fn store(cache: &Cache) -> io::Result<Store> {
    let bundle = Bundle::locate(&cache.bundled())?;
    Ok(Store::bundled(cache.packages(), bundle))
}

/// Which packages a project's dependencies resolve to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Resolve {
    /// The registry's alone, as upstream's commands know them.
    #[default]
    Upstream,
    /// The registry's, the packages bundled with nomi, and the member
    /// packages of the workspace around the project, as nomi's commands
    /// know them.
    Nomi,
}

impl Resolve {
    /// Its name, in the keys of what it resolved.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Resolve::Upstream => "upstream",
            Resolve::Nomi => "nomi",
        }
    }

    /// What `personality`'s commands resolve to.
    pub(crate) fn of(personality: Personality) -> Resolve {
        match personality {
            Personality::Elm => Resolve::Upstream,
            Personality::Nomi => Resolve::Nomi,
        }
    }
}

/// The package store, with the packages bundled with nomi where
/// `resolve` has them.
pub(crate) fn store_for(cache: &Cache, resolve: Resolve) -> Result<Store, Report> {
    if resolve == Resolve::Upstream {
        return Ok(Store::new(cache.packages()));
    }
    store(cache).map_err(|e| {
        one_line(
            "PROBLEM WITH BUNDLED PACKAGES",
            &format!("I could not extract the packages that come with nomi: {e}"),
        )
    })
}

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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use nomi_base::Platform;
    use nomi_codegen_js::platform::{Dom, unavailable};

    #[test]
    fn the_browser_lacks_every_bundled_kernel() {
        let packages = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages");
        let mut kernels = Vec::new();
        for package in fs::read_dir(packages).into_iter().flatten() {
            let dir = package.unwrap().path().join("src/Elm/Kernel");
            for file in fs::read_dir(dir).into_iter().flatten() {
                let file = file.unwrap().path();
                if file.extension().is_some_and(|e| e == "js") {
                    kernels.push(file.file_stem().unwrap().to_string_lossy().into_owned());
                }
            }
        }
        kernels.sort();
        assert_eq!(unavailable(Platform::Browser, Dom::Native), kernels);
        // Kernels share one namespace, keyed by short name; elm/*'s never
        // start with nomi's prefix.
        assert!(kernels.iter().all(|kernel| kernel.starts_with("Nomi")));
    }
}
