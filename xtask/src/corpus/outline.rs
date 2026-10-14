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

pub(crate) fn check(root: &Path) -> Result<()> {
    let corpus = corpus_dir()?;
    let classification = compat::load(root)?;
    let compats = &classification.compats;
    // The manifest, who it is, and the Compats that build it.
    let mut manifests: Vec<(String, Vec<u8>, Compat)> = Vec::new();
    // The manifests again, with the Compat each is built as.
    let mut detected: Vec<(String, Vec<u8>, Compat)> = Vec::new();
    for package in packages::load(root)?.packages {
        let path = packages::package_dir(&corpus, &package).join("elm.json");
        let case = format!("{}@{}", package.name, package.version);
        let excluding = classification
            .package
            .iter()
            .find(|p| p.name == package.name && p.version == package.version);
        let admitted: Vec<Compat> = match excluding {
            Some(p) => compats
                .iter()
                .copied()
                .filter(|c| p.admits.contains(c))
                .collect(),
            None => compats.clone(),
        };
        if admitted.is_empty() {
            continue;
        }
        let text = std::fs::read(&path).with_context(|| {
            format!("reading {}; run `cargo xtask corpus fetch`", path.display())
        })?;
        let newest = *admitted.iter().max().expect("a Compat");
        detected.push((case.clone(), text.clone(), newest));
        for c in admitted {
            manifests.push((case.clone(), text.clone(), c));
        }
    }
    for app in apps::load(root)?.app {
        for &c in compats {
            let text = compat::sandbox_manifest(&classification, &app, c)?;
            let case = format!("app/{}", app.name);
            detected.push((case.clone(), text.clone().into_bytes(), c));
            manifests.push((case, text.into_bytes(), c));
        }
    }
    let mut failures = Vec::new();
    let mut differing = Vec::new();
    for (case, text, compat) in &manifests {
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
            Err(e) => failures.push(format!("{case} on {compat}: unreadable as written: {e:?}")),
        }
        if written != *text && !differing.contains(case) {
            differing.push(case.clone());
        }
    }
    for (case, text, expected) in &detected {
        match Outline::detect(text) {
            Ok((_, compat)) if compat == *expected => {}
            Ok((_, compat)) => failures.push(format!("{case}: detected {compat}, not {expected}")),
            Err(e) => failures.push(format!("{case}: detecting {expected}: {e:?}")),
        }
    }
    println!(
        "{} manifests read; {} projects detected; {} projects written back other than they were",
        manifests.len(),
        detected.len(),
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
