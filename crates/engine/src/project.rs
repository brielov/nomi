//! The project around a directory, as the commands that read or change
//! its dependencies load it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nomi_base::{Compat, Report};
use nomi_project::{Outline, report};

use crate::Vfs;
use crate::console::Personality;
use crate::install::one_line;
use crate::paths::Root;

/// A project's `elm.json`, read.
pub(crate) struct Project {
    pub path: PathBuf,
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
        let path = root.elm_json();
        let src = vfs.read_bytes(&path).map_err(|e| {
            one_line(
                "PROBLEM READING ELM.JSON",
                &format!("I could not read elm.json: {e}"),
            )
        })?;
        let (outline, compat) = Outline::detect(&src)
            .map_err(|e| report::detect(&e, &src, &|c| personality.command(c)))?;
        Ok(Project {
            path,
            outline,
            compat,
        })
    }

    /// Writes `outline` over the project's `elm.json`.
    pub(crate) fn write(&self, vfs: &Vfs, outline: &Outline) -> Result<(), Report> {
        vfs.write(&self.path, outline.write()).map_err(|e| {
            one_line(
                "PROBLEM WRITING ELM.JSON",
                &format!("I could not write elm.json: {e}"),
            )
        })
    }
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
    let Outline::App(app) = outline else {
        return Ok(());
    };
    let dirs: Vec<String> = app.source_dirs.iter().filter_map(|d| d.path()).collect();
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
        let canonical = std::fs::canonicalize(root.join(dir))
            .unwrap_or_else(|_| root.join(dir))
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
