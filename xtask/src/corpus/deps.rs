//! `cargo xtask corpus deps`: whether every corpus app's dependencies
//! check out and download, as each Compat, through nomi's registry code
//! against the live registry. Each archive is checked against the hash
//! the registry publishes for it, as `nomi` does, not against a pin of
//! ours, so this runs by hand, like `corpus pin`, and never in CI.

use std::path::Path;

use anyhow::{Result, bail};
use nomi_project::Outline;
use nomi_registry::{Index, Registry, Store, Web, details};

use super::{apps, compat, parallel};
use crate::oracle::cache_dir;

pub(crate) fn check(root: &Path) -> Result<()> {
    let classification = compat::load(root)?;
    // A nomi cache of the development cache's own, never the user's.
    let cache = cache_dir()?.join("nomi");
    let store = Store::new(cache.join("packages"));
    let http = Web::new();
    let mut failures = Vec::new();
    let mut fetched = 0;
    for app in apps::load(root)?.app {
        for &c in &classification.compats {
            let case = format!("{} on {c}", app.name);
            let text = compat::sandbox_manifest(&classification, &app, c)?;
            let outline = match Outline::read(text.as_bytes(), c) {
                Ok(outline) => outline,
                Err(e) => {
                    failures.push(format!("{case}: {e:?}"));
                    continue;
                }
            };
            let (index, connection) =
                Index::latest(&http, &cache.join("registry").join("index"), c)
                    .map_err(|e| anyhow::anyhow!("{case}: loading the index: {e:?}"))?;
            let mut registry = Registry::new(
                index,
                connection,
                &store,
                cache.join("registry").join("metadata"),
                &http,
                c,
            );
            let solution = match details::verify(&mut registry, c, &outline) {
                Ok(solution) => solution,
                Err(e) => {
                    failures.push(format!("{case}: {e:?}"));
                    continue;
                }
            };
            let releases: Vec<_> = solution
                .iter()
                .map(|(name, details)| (name.clone(), details.version))
                .collect();
            let results = parallel(&releases, |(name, version)| {
                store.fetch(&http, name, *version, c)
            });
            for ((name, version), result) in releases.iter().zip(results) {
                match result {
                    Ok(_) => fetched += 1,
                    Err(e) => failures.push(format!("{case}: {name} {version}: {e:?}")),
                }
            }
        }
    }
    println!("{fetched} releases checked out and downloaded");
    if !failures.is_empty() {
        bail!("{} failures:\n{}", failures.len(), failures.join("\n"));
    }
    Ok(())
}
