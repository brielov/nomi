//! Elm projects as upstream reads and writes them: `elm.json`, package
//! names, versions and constraints; and nomi's own `nomi.toml`. Pure:
//! callers bring the bytes and take the bytes away.

mod constraint;
pub mod json;
mod license;
pub mod manifest;
pub mod module_name;
pub mod outline;
mod package;
pub mod report;
mod scan;
pub mod version;
pub mod workspace;

pub use constraint::{Constraint, ConstraintError, Op};
pub use license::License;
pub use manifest::{Entry, Manifest, ManifestError, ManifestProblem};
pub use module_name::ModuleName;
pub use outline::{DetectError, Outline, OutlineError};
pub use package::Name;
pub use version::Version;

#[cfg(test)]
mod tests;
