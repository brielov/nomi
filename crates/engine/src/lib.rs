//! The effects at the edge of the pure crates (rule R2): the files nomi
//! reads and writes, with the editor's text over them, every path it
//! computes, the process's only thread pool, package downloads on it, and
//! the commands built on them: `format`, and elm-format's command line;
//! and the build's machinery: the module graph, the scheduler that runs it
//! with cancellation, and the artifact cache.

pub mod artifacts;
pub mod bump;
pub mod check;
pub mod console;
pub mod diff;
pub mod elm_format;
pub mod env;
pub mod format;
pub mod graph;
pub mod index;
pub mod init;
pub mod install;
pub mod javascript;
pub mod live;
pub mod make;
pub mod names;
pub mod objects;
pub mod packages;
pub mod paths;
mod pool;
mod project;
pub mod publish;
pub mod remove;
pub mod report;
pub mod schedule;
pub mod test_deps;
pub mod timings;
mod trash;
pub mod tree;
pub mod update;
mod vfs;

pub use nomi_registry::{Http, HttpError, Store, Web};
pub use pool::Pool;
pub use vfs::{Revision, Snapshot, Vfs, canonical};
