//! The project around a directory, as the commands that read or change
//! its dependencies load it.

use std::path::{Path, PathBuf};

use nomi_base::{Compat, Report};
use nomi_project::Outline;

use crate::Vfs;
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
    /// The project around `dir`, whose dependencies `purpose` concerns.
    pub fn load(vfs: &Vfs, dir: &Path, purpose: &str) -> Result<Project, Report> {
        let Some(root) = Root::find(vfs, dir) else {
            return Err(one_line(
                "NO ELM.JSON",
                format!(
                    "I cannot find an elm.json here or in any directory above, so there is no \
                     project {purpose}."
                ),
            ));
        };
        let path = root.elm_json();
        let src = vfs.read_bytes(&path).map_err(|e| {
            one_line(
                "PROBLEM READING ELM.JSON",
                format!("I could not read elm.json: {e}"),
            )
        })?;
        // `toOutlineReport` comes with 4.4a.
        let (outline, compat) = Outline::detect(&src).map_err(|e| {
            one_line(
                "BAD ELM.JSON",
                format!("I ran into a problem with your elm.json file: {e:?}"),
            )
        })?;
        Ok(Project {
            path,
            outline,
            compat,
        })
    }

    /// Writes `outline` over the project's `elm.json`.
    pub fn write(&self, vfs: &Vfs, outline: &Outline) -> Result<(), Report> {
        vfs.write(&self.path, outline.write()).map_err(|e| {
            one_line(
                "PROBLEM WRITING ELM.JSON",
                format!("I could not write elm.json: {e}"),
            )
        })
    }
}
