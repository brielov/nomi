//! The effects at the edge of the pure crates (rule R2): the files nomi
//! reads and writes, with the editor's text over them, every path it
//! computes, the process's only thread pool, package downloads on it, and
//! the commands built on them: `format`, and elm-format's command line;
//! and the build's machinery: the module graph, the scheduler that runs it
//! with cancellation, and the artifact cache.

pub mod artifacts;
pub mod check;
pub mod console;
pub mod elm_format;
mod env;
pub mod format;
pub mod graph;
pub mod index;
pub mod init;
pub mod install;
pub mod javascript;
pub mod make;
pub mod names;
pub mod objects;
pub mod packages;
pub mod paths;
mod pool;
mod project;
pub mod remove;
pub mod report;
pub mod schedule;
pub mod tree;
pub mod update;
mod vfs;

pub use nomi_registry::{Store, Web};
pub use pool::Pool;
pub use vfs::{Revision, Snapshot, Vfs};
