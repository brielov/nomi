//! Shared vocabulary for every nomi crate: file identities, text ranges,
//! line indexes, the diagnostic model, and GHC's character classes and
//! upstream's suggestions, which several crates copy. Depends on no other
//! nomi crate.

mod compat;
pub mod diagnostic;
mod file_id;
pub mod ghc;
mod line_index;
pub mod suggest;

pub use compat::{Compat, UnknownCompat};
pub use diagnostic::{Code, Diagnostic, Doc, Edit, Fix, Label, Report, Severity, Span};
pub use file_id::FileId;
pub use line_index::{ColumnUnit, ElmPosition, LineCol, LineIndex, PositionError};
/// Byte offsets and ranges, shared with rowan so trees need no conversion.
pub use text_size::{TextLen, TextRange, TextSize};

/// The stack every thread that runs core work needs, and gets from the
/// shell. Recursive passes guard themselves with `stacker`, but rowan
/// drops a syntax tree recursively, so this must hold the drop of the
/// deepest tree the parser admits; it is reserved, not committed, so the
/// size costs nothing until used.
pub const STACK_SIZE: usize = 64 * 1024 * 1024;
