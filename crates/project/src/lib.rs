//! Elm projects as upstream reads and writes them: `elm.json`, package
//! names, versions and constraints. Pure: callers bring the bytes and
//! take the bytes away.

pub mod constraint;
pub mod json;
pub mod license;
pub mod module_name;
pub mod outline;
pub mod package;
pub mod report;
mod scan;
pub mod version;

pub use constraint::{Constraint, ConstraintError, Op};
pub use license::License;
pub use module_name::ModuleName;
pub use outline::{DetectError, Outline, OutlineError};
pub use package::Name;
pub use version::Version;

#[cfg(test)]
mod tests;
