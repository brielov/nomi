//! The effects at the edge of the pure crates (rule R2): the files nomi
//! reads and writes, the process's only thread pool, and the commands
//! built on them, `format` first. The build graph, scheduling, caches and
//! cancellation join them from milestone 4.

pub mod format;
mod pool;
mod vfs;

pub use pool::Pool;
pub use vfs::Vfs;
