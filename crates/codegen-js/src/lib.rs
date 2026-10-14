//! JavaScript generation from optimized graphs (architecture §4): the
//! kernel modules' JavaScript, the syntax generated code builds and its
//! printing, the names it uses, and the modes it is generated in.

mod builder;
pub mod debug;
mod expression;
pub mod generate;
pub mod html;
pub mod kernel;
pub mod mode;
mod name;

pub use name::Name;
