//! The project around a directory, as the commands that read or change
//! its dependencies load it.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use nomi_base::{Compat, Report};
use nomi_project::outline::PkgOutline;
use nomi_project::{Outline, report};

use crate::console::Personality;
use crate::paths::{ELM_JSON, Root};
use crate::report::one_line;
use crate::vfs::canonical_or_given;
use crate::{Snapshot, Vfs};

/// A project's `elm.json`, read.
pub(crate) struct Project {
    pub root: Root,
    pub outline: Outline,
    /// The release the outline names.
    pub compat: Compat,
}

impl Project {
    /// The project around `dir`, whose dependencies `purpose` concerns,
    /// for one of nomi's own commands.
    pub(crate) fn load(vfs: &Vfs, dir: &Path, purpose: &str) -> Result<Project, Report> {
        let Some(root) = Root::find(vfs, dir) else {
            return Err(no_outline(purpose));
        };
        Project::read(vfs, &root, Personality::Nomi)
    }

    /// The project at `root`, its problems told as `personality` tells
    /// them.
    pub(crate) fn read(
        vfs: &Vfs,
        root: &Root,
        personality: Personality,
    ) -> Result<Project, Report> {
        Project::decode(root, &elm_json(&vfs.snapshot(), root)?, personality)
    }

    /// The project at `root` whose `elm.json` holds `src`.
    pub(crate) fn decode(
        root: &Root,
        src: &[u8],
        personality: Personality,
    ) -> Result<Project, Report> {
        let (outline, compat) = Outline::detect(src)
            .map_err(|e| report::detect(&e, src, &|c| personality.command(c)))?;
        Ok(Project {
            root: root.clone(),
            outline,
            compat,
        })
    }

    /// Writes `bytes` over the project's `elm.json`.
    pub(crate) fn write(&self, vfs: &Vfs, bytes: impl AsRef<[u8]>) -> Result<(), Report> {
        write_elm_json(vfs, &self.root, bytes)
    }
}

/// Writes `bytes` over the `elm.json` at `root`.
pub(crate) fn write_elm_json(
    vfs: &Vfs,
    root: &Root,
    bytes: impl AsRef<[u8]>,
) -> Result<(), Report> {
    vfs.write(&root.elm_json(), bytes).map_err(|e| {
        one_line(
            "PROBLEM WRITING ELM.JSON",
            &format!("I could not write elm.json: {e}"),
        )
    })
}

/// What the `elm.json` at `root` holds.
pub(crate) fn elm_json(snapshot: &Snapshot, root: &Root) -> Result<Vec<u8>, Report> {
    snapshot.read_bytes(&root.elm_json()).map_err(|e| {
        one_line(
            "PROBLEM READING ELM.JSON",
            &format!("I could not read elm.json: {e}"),
        )
    })
}

/// The outline of the package whose files are in `dir`, as `compat`
/// reads it; none where its `elm.json` isn't a package's.
pub(crate) fn package_outline(dir: &Path, compat: Compat) -> io::Result<Option<PkgOutline>> {
    let bytes = std::fs::read(dir.join(ELM_JSON))?;
    Ok(match Outline::decode(&bytes, compat) {
        Ok(Outline::Pkg(pkg)) => Some(pkg),
        Ok(Outline::App(_)) | Err(_) => None,
    })
}

/// What nomi's own commands say where there is no project for `purpose`.
pub(crate) fn no_outline(purpose: &str) -> Report {
    one_line(
        "NO ELM.JSON",
        &format!(
            "I cannot find an elm.json here or in any directory above, so there is no project \
             {purpose}."
        ),
    )
}

/// The rest of `Outline.read`, which looks at the disk: an application's
/// source directories, as `root` holds them, must exist, and be
/// different directories.
pub(crate) fn check_source_dirs(root: &Path, outline: &Outline) -> Result<(), Report> {
    if let Outline::Pkg(_) = outline {
        return Ok(());
    }
    let dirs = outline.source_dirs();
    let missing: Vec<String> = (dirs.iter())
        .filter(|dir| !root.join(dir).is_dir())
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Err(report::missing_src_dirs(&missing));
    }
    // By canonical path, ordered as Haskell orders strings; the first
    // directory with two names, named as the last two entries name it.
    let mut by_canonical: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for dir in &dirs {
        let canonical = canonical_or_given(&root.join(dir))
            .to_string_lossy()
            .into_owned();
        by_canonical.entry(canonical).or_default().push(dir);
    }
    match by_canonical.iter().find(|(_, names)| names.len() > 1) {
        Some((canonical, names)) => {
            let [.., dir2, dir1] = names[..] else {
                unreachable!("two names at least")
            };
            Err(report::duplicate_src_dirs(canonical, dir1, dir2))
        }
        None => Ok(()),
    }
}
