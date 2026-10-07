//! Shared vocabulary for every nomi crate: file identities, text ranges,
//! line indexes and the diagnostic model. Depends on no other nomi crate.

pub mod diagnostic;
mod file_id;
mod line_index;

pub use diagnostic::{Code, Diagnostic, Doc, Edit, Fix, Label, Severity, Span};
pub use file_id::FileId;
pub use line_index::{ColumnUnit, ElmPosition, LineCol, LineIndex, PositionError};
/// Byte offsets and ranges, shared with rowan so trees need no conversion.
pub use text_size::{TextLen, TextRange, TextSize};
