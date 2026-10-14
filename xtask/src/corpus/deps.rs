//! `cargo xtask corpus deps`: whether every corpus app's dependencies
//! check out and download, as each Compat, through nomi's registry code
//! against the live registry. Each archive is checked against the hash
//! the registry publishes for it, as `nomi` does, not against a pin of
//! ours, so this runs by hand, like `corpus pin`, and never in CI.

use std::path::Path;

use anyhow::{Result, bail};
use nomi_project::Outline;
use nomi_registry::details;

use super::names::Live;
use super::{apps, compat};

pub(crate) fn check(root: &Path) -> Result<()> {
    let classification = compat::load(root)?;
    let live = Live::new()?;
    let mut registries = (classification.compats.iter())
        .map(|&c| Ok((c, live.registry(c)?)))
        .collect::<Result<Vec<_>>>()?;
    let mut failures = Vec::new();
    let mut fetched = 0;
    for app in apps::load(root)?.app {
        for (c, registry) in &mut registries {
            let c = *c;
            let case = format!("{} on {c}", app.name);
            let text = compat::sandbox_manifest(&classification, &app, c)?;
            let outline = match Outline::read(text.as_bytes(), c) {
                Ok(outline) => outline,
                Err(e) => {
                    failures.push(format!("{case}: {e:?}"));
                    continue;
                }
            };
            let solution = match details::verify(registry, c, &outline) {
                Ok(solution) => solution,
                Err(e) => {
                    failures.push(format!("{case}: {e:?}"));
                    continue;
                }
            };
            for download in live.download(&solution, c) {
                match download {
                    Ok(_) => fetched += 1,
                    Err(why) => failures.push(format!("{case}: {why}")),
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
