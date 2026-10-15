//! JavaScript generation from optimized graphs (architecture §4): the
//! kernel modules' JavaScript, the syntax generated code builds and its
//! printing, the names it uses, the modes and targets it is generated
//! in, the runtime a Node program carries, and the names it calls its FFI
//! module's functions by.

mod builder;
pub mod debug;
mod expression;
pub mod ffi;
pub mod generate;
pub mod html;
pub mod kernel;
pub mod mode;
mod name;
pub mod platform;
pub mod reach;
pub mod runtime;

pub use name::Name;
