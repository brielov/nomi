//! `cargo xtask corpus outline`: every corpus `elm.json` read as each
//! Compat that builds the project reads it, and written back. elm wrote
//! most of these files, so writing one back gives its bytes; the rest were
//! edited by hand, in layouts of their own, and must at least come back
//! the same from a second rewrite. Each project must also be detected as
//! the release that builds it: an app as its `elm-version`, a package as
//! the newest release it admits.

use std::path::Path;

use anyhow::{Context, Result, bail};
use nomi_base::Compat;
use nomi_project::Outline;

use super::{apps, compat, corpus_dir, packages};

/// A corpus `elm.json`: its project's case, its text, the Compats that
/// read it, and the one detection finds.
struct Manifest {
    case: String,
    text: Vec<u8>,
    compats: Vec<Compat>,
    detected: Compat,
}

pub(crate) fn check(root: &Path) -> Result<()> {
    let manifests = manifests(root)?;
    let mut read = 0;
    let mut failures = Vec::new();
    let mut differing = Vec::new();
    for Manifest {
        case,
        text,
        compats,
        ..
    } in &manifests
    {
        for compat in compats {
            read += 1;
            let outline = match Outline::read(text, *compat) {
                Ok(outline) => outline,
                Err(e) => {
                    failures.push(format!("{case} on {compat}: {e:?}"));
                    continue;
                }
            };
            let written = outline.write();
            match Outline::read(&written, *compat) {
                Ok(again) if again.write() == written => {}
                Ok(_) => failures.push(format!("{case} on {compat}: a second rewrite differs")),
                Err(e) => {
                    failures.push(format!("{case} on {compat}: unreadable as written: {e:?}"));
                }
            }
            if written != *text && !differing.contains(case) {
                differing.push(case.clone());
            }
        }
    }
    for Manifest {
        case,
        text,
        detected: expected,
        ..
    } in &manifests
    {
        match Outline::detect(text) {
            Ok((_, compat)) if compat == *expected => {}
            Ok((_, compat)) => failures.push(format!("{case}: detected {compat}, not {expected}")),
            Err(e) => failures.push(format!("{case}: detecting {expected}: {e:?}")),
        }
    }
    println!(
        "{read} manifests read; {} projects detected; {} projects written back other than they were",
        manifests.len(),
        differing.len()
    );
    // Their authors' layouts: other indentation, keys in another order,
    // or fields upstream ignores.
    for case in differing.iter().take(5) {
        println!("  {case}");
    }
    if !failures.is_empty() {
        bail!("{} failures:\n{}", failures.len(), failures.join("\n"));
    }
    Ok(())
}

/// Every corpus `elm.json` a Compat builds: a package's once, read by
/// each Compat its range admits and detected as the newest, then an app's
/// as each Compat rewrites it.
fn manifests(root: &Path) -> Result<Vec<Manifest>> {
    let corpus = corpus_dir()?;
    let classification = compat::load(root)?;
    let mut manifests = Vec::new();
    for package in packages::load(root)?.packages {
        let path = packages::package_dir(&corpus, &package).join("elm.json");
        let compats: Vec<Compat> = (classification.compats.iter().copied())
            .filter(|&c| classification.admits(&package.name, &package.version, c))
            .collect();
        let Some(&detected) = compats.iter().max() else {
            continue;
        };
        let text = std::fs::read(&path).with_context(|| {
            format!("reading {}; run `cargo xtask corpus fetch`", path.display())
        })?;
        manifests.push(Manifest {
            case: format!("{}@{}", package.name, package.version),
            text,
            compats,
            detected,
        });
    }
    for app in apps::load(root)?.app {
        for &c in &classification.compats {
            manifests.push(Manifest {
                case: format!("app/{}", app.name),
                text: compat::sandbox_manifest(&classification, &app, c)?.into_bytes(),
                compats: vec![c],
                detected: c,
            });
        }
    }
    Ok(manifests)
}
