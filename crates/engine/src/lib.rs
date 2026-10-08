//! The effects at the edge of the pure crates (rule R2): the files nomi
//! reads and writes, and the process's only thread pool. The build graph,
//! scheduling, caches and cancellation join them from milestone 4.

mod pool;
mod vfs;

pub use pool::Pool;
pub use vfs::Vfs;
