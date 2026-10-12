//! Test support shared by every crate's tests and by xtask: parity
//! ratchets, their expected results, source mutations, command-line
//! transcripts and cases made by editing them. Only ever a dev-dependency of nomi crates; `xtask ci`
//! enforces that.

pub mod edit;
pub mod lsp;
pub mod mutation;
pub mod ratchet;
pub mod transcript;
