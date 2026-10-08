//! The effects at the edge of the pure crates (rule R2): the files nomi
//! reads and writes, every path it computes, the process's only thread
//! pool, package downloads on it, and the commands built on them:
//! `format`, and elm-format's command line. The build graph, scheduling, caches and cancellation
//! join them from milestone 4.

pub mod console;
pub mod elm_format;
mod env;
pub mod format;
pub mod init;
pub mod install;
pub mod packages;
pub mod paths;
mod pool;
mod vfs;

pub use nomi_registry::Web;
pub use pool::Pool;
pub use vfs::Vfs;
